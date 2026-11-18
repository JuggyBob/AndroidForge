#Requires -Version 5.1
<#
.SYNOPSIS
  Verify an in-place installer upgrade preserves user state and can be uninstalled cleanly.

.DESCRIPTION
  Downloads a previous release installer, installs it into a temporary directory,
  creates project/run/state data, upgrades in place to the supplied installer,
  verifies the data survives, then uninstalls and verifies AndroidForge state is
  still present outside the install directory.
#>
param(
    [string]$FromVersion = "0.9.2",
    [Parameter(Mandatory = $true)]
    [string]$ToInstaller,
    [string]$ToVersion = "",
    [string]$EvidenceDir = "",
    [switch]$KeepInstall
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path $ToInstaller)) {
    throw "Target installer not found: $ToInstaller"
}
if (-not $ToVersion) {
    $releaseExe = Join-Path $repoRoot "target\release\androidforge.exe"
    if (Test-Path $releaseExe) {
        $ToVersion = ((& $releaseExe --version) -replace "^androidforge\s+", "").Trim()
    }
    if (-not $ToVersion) { throw "Pass -ToVersion or build target\release\androidforge.exe." }
}
if (-not $EvidenceDir) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $EvidenceDir = Join-Path $repoRoot "target\upgrade-rollback\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$installDir = Join-Path $EvidenceDir "install"
$homeDir = Join-Path $EvidenceDir "home"
$projectDir = Join-Path $EvidenceDir "project"
$downloadDir = Join-Path $EvidenceDir "from-$FromVersion"
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $projectDir ".androidforge") | Out-Null
New-Item -ItemType Directory -Force -Path $downloadDir | Out-Null
Set-Content -Path (Join-Path $projectDir ".androidforge\project.toml") -Encoding utf8 -Value @"
adapter_version = 1

[project]
name = "UpgradeRollback"
package = "com.example.upgrade"
"@
$env:ANDROIDFORGE_HOME = $homeDir
$results = New-Object System.Collections.Generic.List[object]
$index = 0

function Add-Result([string]$Area, [string]$Name, [string]$Status, [string]$Detail, [string]$Evidence) {
    $results.Add([pscustomobject]@{ area = $Area; name = $Name; status = $Status; detail = $Detail; evidence = $Evidence })
    Write-Output ("{0} {1}: {2} ({3})" -f $Status, $Name, $Detail, $Evidence)
}

function Invoke-Installed([string]$Exe, [string[]]$Arguments, [string]$Label) {
    $script:index++
    $name = "{0:D3}-{1}" -f $script:index, $Label
    $out = & $Exe @Arguments 2>&1 | Out-String
    $code = $LASTEXITCODE
    Set-Content -Path (Join-Path $EvidenceDir "$name.txt") -Value $out -Encoding utf8
    return [pscustomobject]@{ ExitCode = $code; Raw = $out; Name = $name }
}

function Install-Msi([string]$Installer, [string]$Directory) {
    $process = Start-Process -FilePath $Installer -ArgumentList @(
        "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOICONS", "/DIR=$Directory"
    ) -Wait -PassThru
    return $process.ExitCode
}

function Uninstall-Installed([string]$Directory) {
    $uninstaller = Get-ChildItem $Directory -Filter "unins*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $uninstaller) { throw "No uninstaller found in $Directory" }
    $process = Start-Process -FilePath $uninstaller.FullName -ArgumentList @(
        "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"
    ) -Wait -PassThru
    return $process.ExitCode
}

$fromInstaller = Join-Path $downloadDir "androidforge-windows-x86_64-setup.exe"
try {
    if (-not (Test-Path $fromInstaller)) {
        & gh release download "v$FromVersion" --pattern "androidforge-windows-x86_64-setup.exe" --dir $downloadDir 2>&1 | Out-String | Set-Content (Join-Path $EvidenceDir "download.txt") -Encoding utf8
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path $fromInstaller)) {
            throw "Could not download the v$FromVersion installer."
        }
    }
    Add-Result "download" "from-$FromVersion" "PASS" "installer available" $fromInstaller

    $fromCode = Install-Msi $fromInstaller $installDir
    Add-Result "install" "from-$FromVersion" $(if ($fromCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$fromCode" ""
    $fromExe = Join-Path $installDir "androidforge.exe"
    if (-not (Test-Path $fromExe)) { throw "Installed CLI is missing." }

    $version = Invoke-Installed $fromExe @("--version") "version-before"
    Add-Result "baseline" "version" $(if ($version.Raw.Trim() -eq "androidforge $FromVersion") { "PASS" } else { "FAIL" }) $version.Raw.Trim() $version.Name

    $started = & $fromExe run start --name pre-upgrade --project $projectDir --json 2>&1 | Out-String
    $startedCode = $LASTEXITCODE
    $startedJson = $started.Substring($started.IndexOf("{")) | ConvertFrom-Json
    $runId = if ($startedJson.run_id) { $startedJson.run_id } else { $startedJson.result.run_id }
    Set-Content -Path (Join-Path $EvidenceDir "pre-upgrade-run.json") -Value $started -Encoding utf8
    Add-Result "baseline" "run-start" $(if ($startedCode -eq 0 -and $runId) { "PASS" } else { "FAIL" }) "run=$runId" "pre-upgrade-run.json"

    $ended = & $fromExe run end --run $runId --status passed --project $projectDir --json 2>&1 | Out-String
    $endedCode = $LASTEXITCODE
    Set-Content -Path (Join-Path $EvidenceDir "pre-upgrade-end.json") -Value $ended -Encoding utf8
    Add-Result "baseline" "run-end" $(if ($endedCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$endedCode" "pre-upgrade-end.json"

    $validate = & $fromExe config validate --project $projectDir --json 2>&1 | Out-String
    $validateJson = $validate.Substring($validate.IndexOf("{")) | ConvertFrom-Json
    Add-Result "baseline" "config" $(if ($validateJson.result.valid) { "PASS" } else { "FAIL" }) "valid=$($validateJson.result.valid)" ""

    $toCode = Install-Msi $ToInstaller $installDir
    Add-Result "upgrade" "install-$ToVersion" $(if ($toCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$toCode" ""
    $toExe = Join-Path $installDir "androidforge.exe"
    $afterVersion = Invoke-Installed $toExe @("--version") "version-after"
    Add-Result "upgrade" "version" $(if ($afterVersion.Raw.Trim() -eq "androidforge $ToVersion") { "PASS" } else { "FAIL" }) $afterVersion.Raw.Trim() $afterVersion.Name

    $shows = & $toExe report show $runId --project $projectDir --json 2>&1 | Out-String
    $showsCode = $LASTEXITCODE
    $showsJson = $shows.Substring($shows.IndexOf("{")) | ConvertFrom-Json
    Set-Content -Path (Join-Path $EvidenceDir "after-upgrade-run.json") -Value $shows -Encoding utf8
    Add-Result "upgrade" "run-preserved" $(if ($showsCode -eq 0 -and $showsJson.result.status -eq "passed") { "PASS" } else { "FAIL" }) "status=$($showsJson.result.status)" "after-upgrade-run.json"

    $validateAfter = & $toExe config validate --project $projectDir --json 2>&1 | Out-String
    $validateAfterJson = $validateAfter.Substring($validateAfter.IndexOf("{")) | ConvertFrom-Json
    Add-Result "upgrade" "config-preserved" $(if ($validateAfterJson.result.valid) { "PASS" } else { "FAIL" }) "valid=$($validateAfterJson.result.valid)" ""

    $homeState = Test-Path (Join-Path $homeDir "state")
    Add-Result "upgrade" "home-state" $(if ($homeState) { "PASS" } else { "FAIL" }) "ANDROIDFORGE_HOME preserved" $homeDir

    if (-not $KeepInstall) {
        $uninstallCode = Uninstall-Installed $installDir
        Start-Sleep -Seconds 2
        Add-Result "uninstall" "exit" $(if ($uninstallCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$uninstallCode" ""
        Add-Result "uninstall" "install-dir-removed" $(if (-not (Test-Path $installDir)) { "PASS" } else { "FAIL" }) "checked $installDir" ""
        Add-Result "uninstall" "state-preserved" $(if (Test-Path $homeDir) { "PASS" } else { "FAIL" }) "user state outside install dir" $homeDir
    }
}
catch {
    Add-Result "harness" "exception" "FAIL" $_.Exception.Message ""
}
finally {
    $summary = [pscustomobject]@{
        evidence_dir = $EvidenceDir
        from_version = $FromVersion
        to_version = $ToVersion
        timestamp = (Get-Date -Format "o")
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        results = $results
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $EvidenceDir "upgrade-summary.json") -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 3)
    if ($summary.fail -gt 0) { exit 1 }
}
