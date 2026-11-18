#Requires -Version 5.1
<#
.SYNOPSIS
  Repeatable long-horizon consistency workload on a disposable headless emulator.

.DESCRIPTION
  Starts its own AVD session, installs a test APK when supplied, then runs a
  configurable force-stop/launch/observe/screenshot/swipe workload with health
  samples, performance runs, reports, a screen recording and bundles. It always
  tries to stop only the session it created and never touches a physical device
  by default.
#>
param(
    [string]$CliPath = "",
    [string]$Project = "A:\Projects\Active\CalcSuite\Internal",
    [string]$Package = "com.calcsuite",
    [string]$Apk = "A:\Projects\Active\CalcSuite\Internal\app\build\outputs\apk\debug\app-debug.apk",
    [string]$Avd = "af_cert_clean",
    [int]$Port = 5558,
    [string]$Owner = "androidforge-long-horizon",
    [int]$MaxCycles = 25,
    [int]$DurationMinutes = 20,
    [string]$EvidenceDir = "",
    [switch]$SkipRecording,
    [switch]$SkipInstall
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
if (-not (Test-Path $Project)) {
    throw "Project directory not found: $Project"
}
if (-not $EvidenceDir) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $EvidenceDir = Join-Path $repoRoot "target\long-horizon\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$homeDir = Join-Path $EvidenceDir "home"
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
$env:ANDROIDFORGE_HOME = $homeDir

$timeline = New-Object System.Collections.Generic.List[string]
$results = New-Object System.Collections.Generic.List[object]
$index = 0
$sessionId = $null
$startedAt = Get-Date
$screenshots = 0
$observations = 0
$recordings = 0
$failures = 0
$lastRunId = $null

function Add-Timeline([string]$Text) {
    $line = "{0} {1}" -f (Get-Date -Format "o"), $Text
    $timeline.Add($line)
}

function Invoke-AfRaw([string[]]$Arguments) {
    $script:index++
    $name = "{0:D4}-{1}" -f $script:index, (($Arguments -join "-") -replace "[^A-Za-z0-9._-]", "_")
    $name = $name.Substring(0, [Math]::Min(120, $name.Length))
    $out = & $CliPath @Arguments 2>&1 | Out-String
    $code = $LASTEXITCODE
    Set-Content -Path (Join-Path $EvidenceDir "$name.stdout.txt") -Value $out -Encoding utf8
    Add-Timeline ("exit={0} argv={1}" -f $code, ($Arguments -join " "))
    return [pscustomobject]@{ ExitCode = $code; Raw = $out; Name = $name }
}

function Invoke-AfJson([string[]]$Arguments) {
    $result = Invoke-AfRaw $Arguments
    $at = $result.Raw.IndexOf("{")
    if ($at -lt 0) { throw "No JSON in $($result.Name): $($result.Raw)" }
    $json = $result.Raw.Substring($at) | ConvertFrom-Json
    return [pscustomobject]@{ ExitCode = $result.ExitCode; Json = $json; Name = $result.Name }
}

function Add-Result([string]$Area, [string]$Name, [string]$Status, [string]$Detail, [string]$Evidence) {
    $results.Add([pscustomobject]@{ area = $Area; name = $Name; status = $Status; detail = $Detail; evidence = $Evidence })
    if ($Status -eq "FAIL") { $script:failures++ }
    Add-Timeline ("{0} {1}: {2} ({3})" -f $Status, $Name, $Detail, $Evidence)
}

function Get-Health([string]$Label) {
    $status = Invoke-AfJson @("session", "status", "--project", $Project, "--session", $sessionId, "--json")
    $health = Invoke-AfJson @("devices", "health", "--serial", $serial, "--json")
    $disk = Get-PSDrive -Name ([System.IO.Path]::GetPathRoot($repoRoot).TrimEnd("\").TrimEnd(":")) -ErrorAction SilentlyContinue
    $obsDir = Join-Path $homeDir "state\observations\$sessionId"
    $obsCount = @(Get-ChildItem $obsDir -Filter "obs-*.json" -ErrorAction SilentlyContinue).Count
    $procs = @(Get-Process -Name androidforge, emulator, qemu-system-x86_64 -ErrorAction SilentlyContinue).Count
    $detail = "state=$($status.Json.result.state) heartbeat=$($status.Json.result.heartbeat_at) obs=$obsCount procs=$procs"
    if ($disk) { $detail += " free_gb=$([Math]::Round($disk.Free / 1GB, 1))" }
    $evidence = "$($status.Name), $($health.Name)"
    if ($status.Json.status -eq "success") {
        Add-Result "health" $Label "PASS" $detail $evidence
    }
    else {
        Add-Result "health" $Label "FAIL" $detail $evidence
    }
}

try {
    Add-Timeline "Long-horizon start. project=$Project avd=$Avd port=$Port cycles=$MaxCycles duration=${DurationMinutes}m"
    $doctor = Invoke-AfJson @("doctor", "--json")
    Add-Result "preflight" "doctor" $(if ($doctor.Json.status -eq "success") { "PASS" } else { "FAIL" }) "version=$($doctor.Json.result.checks | Where-Object { $_.name -eq 'androidforge' } | Select-Object -ExpandProperty detail)" $doctor.Name
    $avds = Invoke-AfJson @("avd", "list", "--json")
    $avdNames = @($avds.Json.result.avds | ForEach-Object { $_.name })
    if ($avdNames -notcontains $Avd) {
        throw "AVD '$Avd' was not found. Create a disposable AVD or pass -Avd."
    }

    $start = Invoke-AfJson @("session", "ensure", "--project", $Project, "--avd", $Avd, "--port", "$Port", "--headless", "--owner", $Owner, "--ttl", "4h", "--prepare", "--json")
    if ($start.Json.status -ne "success") {
        Add-Result "session" "start" "FAIL" "code=$($start.Json.error.code)" $start.Name
        throw "Could not start the emulator: $($start.Json.error.message)"
    }
    $sessionId = if ($start.Json.result.session_id) { $start.Json.result.session_id } else { $start.Json.session_id }
    $serial = $start.Json.result.device.serial
    Add-Result "session" "start" "PASS" "session=$sessionId serial=$serial avd=$Avd" $start.Name

    if (-not $SkipInstall -and (Test-Path $Apk)) {
        $inspect = Invoke-AfJson @("apk", "inspect", $Apk, "--json")
        Add-Result "app" "apk-inspect" $(if ($inspect.Json.status -eq "success") { "PASS" } else { "FAIL" }) "package=$($inspect.Json.result.package) version=$($inspect.Json.result.version_name)" $inspect.Name
        $install = Invoke-AfJson @("app", "install", $Apk, "--project", $Project, "--session", $sessionId, "--json")
        Add-Result "app" "install" $(if ($install.Json.status -eq "success") { "PASS" } else { "FAIL" }) "code=$($install.Json.error.code)" $install.Name
        $skip = Invoke-AfJson @("app", "install", $Apk, "--only-if-changed", "--project", $Project, "--session", $sessionId, "--json")
        Add-Result "app" "install-only-if-changed" $(if ($skip.Json.status -eq "success") { "PASS" } else { "FAIL" }) "skipped=$($skip.Json.result.skipped) reason=$($skip.Json.result.reason)" $skip.Name
    }
    else {
        Add-Result "app" "install" "N/A" "APK not present or -SkipInstall" ""
    }

    $launch = Invoke-AfJson @("app", "launch", $Package, "--project", $Project, "--session", $sessionId, "--json")
    Add-Result "app" "launch" $(if ($launch.Json.status -eq "success") { "PASS" } else { "FAIL" }) "foreground=$($launch.Json.result.foreground_app.package)" $launch.Name
    $dismiss = Invoke-AfJson @("ui", "dismiss-overlays", "--project", $Project, "--session", $sessionId, "--json")
    Add-Result "ui" "dismiss-overlays" $(if ($dismiss.Json.status -eq "success") { "PASS" } else { "FAIL" }) "dismissed=$($dismiss.Json.result.dismissed.Count) remaining=$($dismiss.Json.result.remaining_overlays)" $dismiss.Name

    $deadline = (Get-Date).AddMinutes([Math]::Max(1, $DurationMinutes))
    $cycle = 0
    while ($cycle -lt $MaxCycles -and (Get-Date) -lt $deadline) {
        $cycle++
        $cycleStarted = Get-Date
        $force = Invoke-AfJson @("app", "force-stop", $Package, "--project", $Project, "--session", $sessionId, "--json")
        $launch = Invoke-AfJson @("app", "launch", $Package, "--project", $Project, "--session", $sessionId, "--json")
        $observe = Invoke-AfJson @("ui", "observe", "--project", $Project, "--session", $sessionId, "--json")
        if ($observe.Json.status -eq "success") { $script:observations++ }
        $shot = Invoke-AfJson @("capture", "screenshot", "--project", $Project, "--session", $sessionId, "--label", "cycle-$cycle", "--json")
        if ($shot.Json.status -eq "success") {
            $script:screenshots++
            $lastRunId = if ($shot.Json.result.run_id) { $shot.Json.result.run_id } else { $shot.Json.run_id }
        }
        $swipe = Invoke-AfJson @("ui", "swipe", "540", "1800", "540", "900", "--project", $Project, "--session", $sessionId, "--json")
        $homeResult = Invoke-AfJson @("ui", "home", "--project", $Project, "--session", $sessionId, "--json")
        $ok = $force.Json.status -eq "success" -and $launch.Json.status -eq "success" -and $observe.Json.status -eq "success" -and $shot.Json.status -eq "success" -and $swipe.Json.status -eq "success" -and $homeResult.Json.status -eq "success"
        Add-Result "cycle" "$cycle" $(if ($ok) { "PASS" } else { "FAIL" }) ("elapsed={0:N0}ms obs=$($observe.Json.result.actionable_total) token=$($observe.Json.result.token)" -f ((Get-Date) - $cycleStarted).TotalMilliseconds) "$($observe.Name), $($shot.Name)"
        if ($cycle % 5 -eq 0) {
            Get-Health "cycle-$cycle"
            $perf = Invoke-AfJson @("diagnostics", "performance", $Package, "--settle", "1s", "--project", $Project, "--session", $sessionId, "--json")
            Add-Result "diagnostics" "performance-$cycle" $(if ($perf.Json.status -eq "success") { "PASS" } else { "FAIL" }) "startup=$($perf.Json.result.startup.total_time_ms) jank=$($perf.Json.result.jank.janky_percent)" $perf.Name
        }
    }

    if (-not $SkipRecording) {
        $recordStart = Invoke-AfJson @("capture", "record", "start", "--time-limit", "15", "--project", $Project, "--session", $sessionId, "--json")
        if ($recordStart.Json.status -eq "success") {
            $recordingRun = if ($recordStart.Json.result.run_id) { $recordStart.Json.result.run_id } else { $recordStart.Json.run_id }
            for ($i = 0; $i -lt 3; $i++) {
                Invoke-AfJson @("ui", "swipe", "540", "1800", "540", "900", "--project", $Project, "--session", $sessionId, "--json") | Out-Null
                Start-Sleep -Seconds 4
            }
            $recordStop = Invoke-AfJson @("capture", "record", "stop", "--project", $Project, "--session", $sessionId, "--json")
            if ($recordStop.Json.status -eq "success") {
                $script:recordings++
                Add-Result "capture" "record" "PASS" "size=$($recordStop.Json.result.size_bytes) duration=$($recordStop.Json.result.duration_ms)ms" "$($recordStart.Name), $($recordStop.Name)"
                $lastRunId = $recordingRun
            }
            else {
                Add-Result "capture" "record" "FAIL" "code=$($recordStop.Json.error.code)" $recordStop.Name
            }
        }
        else {
            Add-Result "capture" "record" "FAIL" "code=$($recordStart.Json.error.code)" $recordStart.Name
        }
    }

    if ($lastRunId) {
        $list = Invoke-AfJson @("report", "list", "--project", $Project, "--limit", "10", "--json")
        Add-Result "report" "list" $(if ($list.Json.status -eq "success") { "PASS" } else { "FAIL" }) "runs=$(@($list.Json.result.runs).Count)" $list.Name
        $bundlePath = Join-Path $EvidenceDir "evidence.zip"
        $bundle = Invoke-AfJson @("report", "bundle", $lastRunId, "--project", $Project, "--output", $bundlePath, "--json")
        Add-Result "report" "bundle" $(if ($bundle.Json.status -eq "success") { "PASS" } else { "FAIL" }) "entries=$($bundle.Json.result.manifest.entries.Count) verified=$($bundle.Json.result.verified)" $bundle.Name
    }

    Get-Health "final"
    $elapsed = (Get-Date) - $startedAt
    Add-Result "summary" "duration" "PASS" ("{0:N0}s cycles={1} commands={2}" -f $elapsed.TotalSeconds, $cycle, $index) ""
    if ($elapsed.TotalSeconds -lt ([Math]::Max(1, $DurationMinutes) * 60)) {
        Add-Result "summary" "duration-target" "N/A" "cycle cap reached before the wall-clock target; reported honestly" ""
    }
}
catch {
    Add-Result "harness" "exception" "FAIL" $_.Exception.Message ""
}
finally {
    if ($sessionId) {
        $stop = Invoke-AfRaw @("session", "stop", "--project", $Project, "--session", $sessionId, "--owner", $Owner, "--json")
        if ($stop.ExitCode -eq 0) {
            Add-Result "cleanup" "session-stop" "PASS" "stopped $sessionId" $stop.Name
        }
        else {
            Add-Result "cleanup" "session-stop" "FAIL" "exit=$($stop.ExitCode)" $stop.Name
        }
    }
    $summary = [pscustomobject]@{
        evidence_dir = $EvidenceDir
        project = $Project
        avd = $Avd
        serial = $(if ($serial) { $serial } else { $null })
        session_id = $sessionId
        started_at = $startedAt.ToString("o")
        duration_seconds = [Math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1)
        commands = $index
        cycles = $cycle
        observations = $observations
        screenshots = $screenshots
        recordings = $recordings
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        blocked = @($results | Where-Object { $_.status -eq "BLOCKED" }).Count
        na = @($results | Where-Object { $_.status -eq "N/A" }).Count
        results = $results
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $EvidenceDir "long-horizon-summary.json") -Encoding utf8
    Set-Content -Path (Join-Path $EvidenceDir "TIMELINE.txt") -Value $timeline -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 4)
    if ($summary.fail -gt 0) { exit 1 }
}
