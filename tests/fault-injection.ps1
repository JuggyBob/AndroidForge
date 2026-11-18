#Requires -Version 5.1
<#
.SYNOPSIS
  CLI-level fault-injection checks for state corruption, stale locks and
  concurrent writers.

.DESCRIPTION
  Uses an isolated ANDROIDFORGE_HOME and temporary project. It never touches a
  device. It verifies that malformed state produces structured JSON errors,
  stale lock files do not block new work, corrupt session/report files do not
  crash list commands, and concurrent run writers receive distinct ids.
#>
param(
    [string]$CliPath = "",
    [string]$EvidenceDir = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $CliPath) {
    $candidates = @(
        (Join-Path $repoRoot "target\release\androidforge.exe"),
        (Join-Path $repoRoot "target\debug\androidforge.exe")
    )
    $CliPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $CliPath -or -not (Test-Path $CliPath)) {
    throw "androidforge.exe was not found. Build the workspace or pass -CliPath."
}
if (-not $EvidenceDir) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $EvidenceDir = Join-Path $repoRoot "target\fault-injection\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$homeDir = Join-Path $EvidenceDir "home"
$projectDir = Join-Path $EvidenceDir "project"
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $projectDir ".androidforge") | Out-Null
Set-Content -Path (Join-Path $projectDir ".androidforge\project.toml") -Encoding utf8 -Value @"
adapter_version = 1

[project]
name = "FaultInjection"
package = "com.example.faults"
"@
$env:ANDROIDFORGE_HOME = $homeDir
$results = New-Object System.Collections.Generic.List[object]
$index = 0

function Add-Result([string]$Area, [string]$Name, [string]$Status, [string]$Detail, [string]$Evidence) {
    $results.Add([pscustomobject]@{ area = $Area; name = $Name; status = $Status; detail = $Detail; evidence = $Evidence })
    Write-Output ("{0} {1}: {2} ({3})" -f $Status, $Name, $Detail, $Evidence)
}

function Invoke-Af([string[]]$Arguments, [string]$Label) {
    $script:index++
    $name = "{0:D3}-{1}" -f $script:index, $Label
    $out = & $CliPath @Arguments 2>&1 | Out-String
    $code = $LASTEXITCODE
    Set-Content -Path (Join-Path $EvidenceDir "$name.txt") -Value $out -Encoding utf8
    $json = $null
    $at = $out.IndexOf("{")
    if ($at -ge 0) {
        try { $json = $out.Substring($at) | ConvertFrom-Json } catch { $json = $null }
    }
    return [pscustomobject]@{ ExitCode = $code; Raw = $out; Json = $json; Name = $name }
}

try {
    $doctor = Invoke-Af @("doctor", "--json") "doctor"
    Add-Result "preflight" "doctor" $(if ($doctor.ExitCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$($doctor.ExitCode)" $doctor.Name

    # Malformed project adapter: unknown nested field.
    $adapter = Join-Path $projectDir ".androidforge\project.toml"
    $original = Get-Content $adapter -Raw
    Set-Content -Path $adapter -Value ($original + "unknown_key = true`n") -Encoding utf8
    $unknown = Invoke-Af @("config", "validate", "--project", $projectDir, "--json") "config-unknown"
    $structured = $unknown.Json -and $unknown.Json.error.code -eq "E_CONFIG_INVALID"
    Add-Result "adapter" "unknown-field" $(if ($structured) { "PASS" } else { "FAIL" }) "exit=$($unknown.ExitCode) code=$($unknown.Json.error.code)" $unknown.Name
    Set-Content -Path $adapter -Value $original -Encoding utf8

    # Invalid retention range is a structured validation failure.
    $retention = $original + "`n[storage]`nobservation_retention = 1`n"
    Set-Content -Path $adapter -Value $retention -Encoding utf8
    $invalid = Invoke-Af @("config", "validate", "--project", $projectDir, "--json") "config-retention"
    $retentionOk = $invalid.Json -and $invalid.Json.result.valid -eq $false -and $invalid.Json.exit_code -eq 8
    Add-Result "adapter" "invalid-retention" $(if ($retentionOk) { "PASS" } else { "FAIL" }) "valid=$($invalid.Json.result.valid) exit=$($invalid.ExitCode)" $invalid.Name
    Set-Content -Path $adapter -Value $original -Encoding utf8

    # Stale global lock contents must not block a new command.
    $locks = Join-Path $homeDir "state\locks"
    New-Item -ItemType Directory -Force -Path $locks | Out-Null
    Set-Content -Path (Join-Path $locks ".global.lock") -Value "stale junk from a killed process" -Encoding utf8
    $sessionList = Invoke-Af @("session", "list", "--json") "session-list-stale-lock"
    Add-Result "locks" "stale-lock" $(if ($sessionList.ExitCode -eq 0 -and $sessionList.Json.status -eq "success") { "PASS" } else { "FAIL" }) "exit=$($sessionList.ExitCode)" $sessionList.Name

    # Corrupt session metadata must not crash a list command.
    $sessions = Join-Path $homeDir "state\sessions"
    New-Item -ItemType Directory -Force -Path $sessions | Out-Null
    Set-Content -Path (Join-Path $sessions "ses-corrupt.json") -Value "{not-json" -Encoding utf8
    $corruptSession = Invoke-Af @("session", "list", "--json") "session-list-corrupt"
    Add-Result "sessions" "corrupt-json" $(if ($corruptSession.ExitCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$($corruptSession.ExitCode)" $corruptSession.Name

    # Two independent run writers must both succeed with distinct ids.
    $outA = Join-Path $EvidenceDir "run-a.json"
    $outB = Join-Path $EvidenceDir "run-b.json"
    $procA = Start-Process -FilePath $CliPath -ArgumentList @("run", "start", "--name", "concurrent-a", "--project", $projectDir, "--json") -RedirectStandardOutput $outA -RedirectStandardError "$outA.err" -PassThru -WindowStyle Hidden
    $procB = Start-Process -FilePath $CliPath -ArgumentList @("run", "start", "--name", "concurrent-b", "--project", $projectDir, "--json") -RedirectStandardOutput $outB -RedirectStandardError "$outB.err" -PassThru -WindowStyle Hidden
    $procA.WaitForExit(60000) | Out-Null
    $procB.WaitForExit(60000) | Out-Null
    $jsonA = Get-Content $outA -Raw | ConvertFrom-Json
    $jsonB = Get-Content $outB -Raw | ConvertFrom-Json
    $idA = if ($jsonA.run_id) { $jsonA.run_id } else { $jsonA.result.run_id }
    $idB = if ($jsonB.run_id) { $jsonB.run_id } else { $jsonB.result.run_id }
    $distinct = $idA -and $idB -and $idA -ne $idB
    Add-Result "concurrency" "run-start" $(if ($distinct) { "PASS" } else { "FAIL" }) "a=$idA b=$idB" "run-a.json, run-b.json"
    if ($idA) { Invoke-Af @("run", "end", "--run", $idA, "--status", "passed", "--project", $projectDir, "--json") "run-a-end" | Out-Null }
    if ($idB) { Invoke-Af @("run", "end", "--run", $idB, "--status", "passed", "--project", $projectDir, "--json") "run-b-end" | Out-Null }

    # Corrupt one report and verify report show returns structured JSON, not a panic.
    $runToCorrupt = $idA
    $runDir = Join-Path $projectDir ".artifacts\runs\$runToCorrupt"
    if (Test-Path (Join-Path $runDir "report.json")) {
        Set-Content -Path (Join-Path $runDir "report.json") -Value "{corrupt" -Encoding utf8
        $show = Invoke-Af @("report", "show", $runToCorrupt, "--project", $projectDir, "--json") "report-show-corrupt"
        $showOk = $show.Json -and $show.Json.status -eq "error" -and $show.Json.error.code
        Add-Result "reports" "corrupt-report" $(if ($showOk) { "PASS" } else { "FAIL" }) "exit=$($show.ExitCode) code=$($show.Json.error.code)" $show.Name
    }
    else {
        Add-Result "reports" "corrupt-report" "N/A" "run report not found at $runDir" ""
    }

    # A second writer pair after corruption must still create valid runs.
    $lateA = Invoke-Af @("run", "start", "--name", "post-corruption", "--project", $projectDir, "--json") "run-post-corruption"
    Add-Result "recovery" "post-corruption-run" $(if ($lateA.ExitCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$($lateA.ExitCode)" $lateA.Name
    $lateId = if ($lateA.Json.run_id) { $lateA.Json.run_id } else { $lateA.Json.result.run_id }
    if ($lateId) { Invoke-Af @("run", "end", "--run", $lateId, "--status", "passed", "--project", $projectDir, "--json") "run-post-corruption-end" | Out-Null }

    Add-Result "limits" "disk-full" "N/A" "not simulated; atomic-write and truncated-state tests cover the failure shape" ""
    Add-Result "limits" "read-only-state" "N/A" "ACL mutation is host-specific; not performed by this harness" ""
}
catch {
    Add-Result "harness" "exception" "FAIL" $_.Exception.Message ""
}
finally {
    $summary = [pscustomobject]@{
        evidence_dir = $EvidenceDir
        timestamp = (Get-Date -Format "o")
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        blocked = @($results | Where-Object { $_.status -eq "BLOCKED" }).Count
        na = @($results | Where-Object { $_.status -eq "N/A" }).Count
        results = $results
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $EvidenceDir "fault-injection-summary.json") -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 3)
    if ($summary.fail -gt 0) { exit 1 }
}
