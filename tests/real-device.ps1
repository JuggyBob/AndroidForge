#Requires -Version 5.1
<#
.SYNOPSIS
  Read-only/safe compatibility harness for a physical Android device.

.DESCRIPTION
  Attaches a temporary AndroidForge session, records vendor/device properties,
  exercises read-only observation and screenshot paths, performs only benign UI
  navigation (Settings, scroll, Back, Home), optionally installs one explicitly
  supplied test APK, and releases the session.

  The harness never factory-resets, clears app data, uninstalls a pre-existing
  app, restores data, loads a snapshot, or changes security/network/time
  settings. It refuses to run against an emulator serial.
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Serial,
    [string]$CliPath = "",
    [string]$Owner = "androidforge-real-device",
    [string]$EvidenceDir = "",
    [switch]$AllowTestApk,
    [string]$TestApk = "",
    [string]$TestPackage = "",
    [int]$SoakCycles = 3,
    [int]$ScreenshotCount = 3
)

$ErrorActionPreference = "Stop"

if ($Serial -like "emulator-*") {
    throw "This harness is for a physical device; '$Serial' is an emulator serial."
}

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
$adb = Join-Path $env:LOCALAPPDATA "Android\Sdk\platform-tools\adb.exe"
if (-not (Test-Path $adb)) {
    $adb = "adb"
}
if (-not $EvidenceDir) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $EvidenceDir = Join-Path $repoRoot "target\real-device\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$homeDir = Join-Path $EvidenceDir "home"
$projectDir = Join-Path $EvidenceDir "project"
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $projectDir ".androidforge") | Out-Null
Set-Content -Path (Join-Path $projectDir ".androidforge\project.toml") -Encoding utf8 -Value @"
adapter_version = 1

[project]
name = "RealDeviceHarness"
package = "com.android.settings"
"@

$env:ANDROIDFORGE_HOME = $homeDir
$timeline = New-Object System.Collections.Generic.List[string]
$results = New-Object System.Collections.Generic.List[object]
$index = 0
$sessionId = $null
$failed = 0

function Add-Timeline([string]$Text) {
    $line = "{0} {1}" -f (Get-Date -Format "o"), $Text
    $timeline.Add($line)
    Add-Content -Path (Join-Path $EvidenceDir "TIMELINE.txt") -Value $line
}

function Invoke-AfRaw([string[]]$Arguments) {
    $script:index++
    $name = "{0:D3}-{1}" -f $script:index, (($Arguments -join "-") -replace "[^A-Za-z0-9._-]", "_")
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
    if ($at -lt 0) {
        throw "No JSON in $($result.Name): $($result.Raw)"
    }
    try {
        $json = $result.Raw.Substring($at) | ConvertFrom-Json
    }
    catch {
        throw "Invalid JSON in $($result.Name): $($_.Exception.Message)"
    }
    return [pscustomobject]@{ ExitCode = $result.ExitCode; Json = $json; Name = $result.Name }
}

function Add-Result([string]$Area, [string]$Name, [string]$Status, [string]$Detail, [string]$Evidence) {
    $results.Add([pscustomobject]@{
        area = $Area
        name = $Name
        status = $Status
        detail = $Detail
        evidence = $Evidence
    })
    if ($Status -eq "FAIL") { $script:failed++ }
    Add-Timeline ("{0} {1}: {2} ({3})" -f $Status, $Name, $Detail, $Evidence)
}

function Get-Prop([string]$Name) {
    $output = & $adb -s $Serial shell getprop $Name 2>&1 | Out-String
    return $output.Trim()
}

try {
    Add-Timeline "Starting physical-device harness for $Serial"
    $devices = & $adb devices -l 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "adb-devices.txt") -Value $devices -Encoding utf8
    $line = ($devices -split "`r?`n" | Where-Object { $_ -match [regex]::Escape($Serial) } | Select-Object -First 1)
    if (-not $line) {
        Add-Result "preflight" "device" "BLOCKED" "serial not present in adb devices" "adb-devices.txt"
        throw "Device $Serial is not connected."
    }
    if ($line -notmatch "\sdevice\s") {
        Add-Result "preflight" "authorization" "BLOCKED" "device is not authorized/online: $line" "adb-devices.txt"
        throw "Device $Serial is not authorized. Unlock it and accept the USB debugging prompt."
    }
    Add-Result "preflight" "device" "PASS" $line "adb-devices.txt"

    $props = [ordered]@{}
    foreach ($name in @(
        "ro.product.manufacturer", "ro.product.model", "ro.product.device",
        "ro.product.name", "ro.build.version.release", "ro.build.version.sdk",
        "ro.build.fingerprint", "ro.build.version.incremental", "ro.product.cpu.abi",
        "ro.boot.hardware", "ro.kernel.qemu", "ro.debuggable",
        "ro.mi.os.version.name", "ro.miui.ui.version.name"
    )) {
        $props[$name] = Get-Prop $name
    }
    $props | ConvertTo-Json | Set-Content -Path (Join-Path $EvidenceDir "device-properties.json") -Encoding utf8
    Add-Result "preflight" "properties" "PASS" "model=$($props['ro.product.model']) android=$($props['ro.build.version.release']) hyperos=$($props['ro.mi.os.version.name'])" "device-properties.json"

    $window = & $adb -s $Serial shell dumpsys window 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "keyguard-window.txt") -Value $window -Encoding utf8
    $locked = $window -match "mDreamingLockscreen=true" -or $window -match "mShowingLockscreen=true" -or $window -match "isStatusBarKeyguard=true"
    if ($locked) {
        Add-Result "preflight" "keyguard" "BLOCKED" "device is locked; unlock it and rerun" "keyguard-window.txt"
        throw "Device $Serial is locked. Unlock it and rerun the harness."
    }
    Add-Result "preflight" "keyguard" "PASS" "unlocked" "keyguard-window.txt"

    $battery = & $adb -s $Serial shell dumpsys battery 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "battery.txt") -Value $battery -Encoding utf8
    $storage = & $adb -s $Serial shell df -h /data 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "storage.txt") -Value $storage -Encoding utf8
    Add-Result "preflight" "health" "PASS" "battery/storage captured" "battery.txt, storage.txt"

    $health = Invoke-AfJson @("devices", "health", "--serial", $Serial, "--json")
    if ($health.Json.status -eq "success") {
        Add-Result "preflight" "devices.health" "PASS" "structured health returned" $health.Name
    }
    else {
        Add-Result "preflight" "devices.health" "FAIL" "status=$($health.Json.status) code=$($health.Json.error.code)" $health.Name
    }

    $existing = Invoke-AfJson @("session", "list", "--json")
    if ($existing.Json.status -eq "success") {
        foreach ($candidate in @($existing.Json.result.sessions)) {
            if ($candidate.serial -eq $Serial -and $candidate.owner -eq $Owner -and $candidate.state -notin @("released", "failed", "abandoned")) {
                Add-Result "session" "stale-session" "PASS" "releasing own stale session $($candidate.id)" $existing.Name
                $releaseStale = Invoke-AfJson @("session", "release", "--project", $projectDir, "--session", $candidate.id, "--owner", $Owner, "--json")
                if ($releaseStale.Json.status -ne "success") {
                    Add-Result "session" "stale-session-release" "FAIL" "code=$($releaseStale.Json.error.code)" $releaseStale.Name
                }
            }
        }
    }

    $attach = Invoke-AfJson @("session", "attach", "--serial", $Serial, "--project", $projectDir, "--owner", $Owner, "--json")
    if ($attach.Json.status -ne "success") {
        $status = if ($attach.Json.error.code -eq "E_DEVICE_BUSY") { "BLOCKED" } else { "FAIL" }
        Add-Result "session" "attach" $status "status=$($attach.Json.status) code=$($attach.Json.error.code)" $attach.Name
        throw "Could not attach a session: $($attach.Json.error.message)"
    }
    $sessionId = if ($attach.Json.result.session_id) { $attach.Json.result.session_id } else { $attach.Json.session_id }
    Add-Result "session" "attach" "PASS" "session=$sessionId" $attach.Name

    $status = Invoke-AfJson @("session", "status", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($status.Json.status -eq "success") {
        Add-Result "session" "status" "PASS" "state=$($status.Json.result.state) serial=$($status.Json.result.serial)" $status.Name
    }
    else {
        Add-Result "session" "status" "FAIL" "code=$($status.Json.error.code)" $status.Name
    }

    $context = Invoke-AfJson @("context", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($context.Json.status -eq "success") {
        Add-Result "read-only" "context" "PASS" "device=$($context.Json.result.device.serial) package=$($context.Json.result.project.package)" $context.Name
    }
    else {
        Add-Result "read-only" "context" "FAIL" "code=$($context.Json.error.code)" $context.Name
    }

    for ($i = 1; $i -le [Math]::Max(1, $ScreenshotCount); $i++) {
        $shot = Invoke-AfJson @("capture", "screenshot", "--project", $projectDir, "--session", $sessionId, "--label", "real-device-$i", "--json")
        if ($shot.Json.status -eq "success") {
            Add-Result "read-only" "screenshot-$i" "PASS" "$($shot.Json.result.width)x$($shot.Json.result.height) $($shot.Json.result.artifact.size_bytes) bytes" $shot.Name
        }
        else {
            Add-Result "read-only" "screenshot-$i" "FAIL" "code=$($shot.Json.error.code)" $shot.Name
        }
    }

    $observe = Invoke-AfJson @("ui", "observe", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($observe.Json.status -eq "success") {
        Add-Result "read-only" "ui.observe" "PASS" "token=$($observe.Json.result.token) actionable=$($observe.Json.result.actionable_total)" $observe.Name
    }
    else {
        Add-Result "read-only" "ui.observe" "FAIL" "code=$($observe.Json.error.code)" $observe.Name
    }

    $launch = Invoke-AfJson @("app", "launch", "com.android.settings", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($launch.Json.status -eq "success") {
        Add-Result "ui" "launch-settings" "PASS" "Settings foreground" $launch.Name
    }
    else {
        Add-Result "ui" "launch-settings" "FAIL" "code=$($launch.Json.error.code)" $launch.Name
    }
    Start-Sleep -Seconds 1
    $settingsObserve = Invoke-AfJson @("ui", "observe", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($settingsObserve.Json.status -eq "success") {
        Add-Result "ui" "observe-settings" "PASS" "token=$($settingsObserve.Json.result.token) nodes=$($settingsObserve.Json.result.hierarchy.node_count)" $settingsObserve.Name
    }
    else {
        Add-Result "ui" "observe-settings" "FAIL" "code=$($settingsObserve.Json.error.code)" $settingsObserve.Name
    }

    $swipe = Invoke-AfJson @("ui", "swipe", "540", "1800", "540", "900", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($swipe.Json.status -eq "success") {
        Add-Result "ui" "swipe" "PASS" "scrolled Settings list" $swipe.Name
    }
    else {
        Add-Result "ui" "swipe" "FAIL" "code=$($swipe.Json.error.code)" $swipe.Name
    }
    $back = Invoke-AfJson @("ui", "back", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($back.Json.status -eq "success") {
        Add-Result "ui" "back" "PASS" "returned from Settings" $back.Name
    }
    else {
        Add-Result "ui" "back" "FAIL" "code=$($back.Json.error.code)" $back.Name
    }
    $homeResult = Invoke-AfJson @("ui", "home", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($homeResult.Json.status -eq "success") {
        Add-Result "ui" "home" "PASS" "returned to Home" $homeResult.Name
    }
    else {
        Add-Result "ui" "home" "FAIL" "code=$($homeResult.Json.error.code)" $homeResult.Name
    }

    $collect = Invoke-AfJson @("diagnostics", "collect", "--project", $projectDir, "--session", $sessionId, "--package", "com.android.settings", "--json")
    if ($collect.Json.status -eq "success") {
        Add-Result "diagnostics" "collect" "PASS" "artifacts=$($collect.Json.artifacts.Count)" $collect.Name
    }
    else {
        Add-Result "diagnostics" "collect" "FAIL" "code=$($collect.Json.error.code)" $collect.Name
    }

    $logcat = Invoke-AfJson @("diagnostics", "logcat", "start", "--project", $projectDir, "--session", $sessionId, "--json")
    if ($logcat.Json.status -eq "success") {
        Start-Sleep -Seconds 8
        $logStop = Invoke-AfJson @("diagnostics", "logcat", "stop", "--project", $projectDir, "--session", $sessionId, "--json")
        if ($logStop.Json.status -eq "success") {
            Add-Result "diagnostics" "logcat" "PASS" "bounded 8s capture" "$($logcat.Name), $($logStop.Name)"
        }
        else {
            Add-Result "diagnostics" "logcat.stop" "FAIL" "code=$($logStop.Json.error.code)" $logStop.Name
        }
    }
    else {
        Add-Result "diagnostics" "logcat.start" "FAIL" "code=$($logcat.Json.error.code)" $logcat.Name
    }

    if ($AllowTestApk) {
        if (-not $TestApk -or -not (Test-Path $TestApk) -or -not $TestPackage) {
            Add-Result "test-apk" "install" "BLOCKED" "-AllowTestApk requires -TestApk and -TestPackage" ""
        }
        else {
            $info = Invoke-AfJson @("app", "info", $TestPackage, "--project", $projectDir, "--session", $sessionId, "--json")
            if ($info.Json.status -eq "success") {
                Add-Result "test-apk" "pre-existing-package" "BLOCKED" "$TestPackage is already installed; refusing to uninstall or clear it" $info.Name
            }
            else {
                $install = Invoke-AfJson @("app", "install", $TestApk, "--project", $projectDir, "--session", $sessionId, "--allow-physical-mutation", "--json")
                if ($install.Json.status -eq "success") {
                    Add-Result "test-apk" "install" "PASS" "installed $TestPackage" $install.Name
                    $testLaunch = Invoke-AfJson @("app", "launch", $TestPackage, "--project", $projectDir, "--session", $sessionId, "--json")
                    if ($testLaunch.Json.status -eq "success") {
                        Add-Result "test-apk" "launch" "PASS" "test package foreground" $testLaunch.Name
                    }
                    else {
                        Add-Result "test-apk" "launch" "FAIL" "code=$($testLaunch.Json.error.code)" $testLaunch.Name
                    }
                    $testUninstall = Invoke-AfJson @("app", "uninstall", $TestPackage, "--yes", "--allow-physical-mutation", "--project", $projectDir, "--session", $sessionId, "--json")
                    if ($testUninstall.Json.status -eq "success") {
                        Add-Result "test-apk" "uninstall" "PASS" "removed only $TestPackage" $testUninstall.Name
                    }
                    else {
                        Add-Result "test-apk" "uninstall" "FAIL" "code=$($testUninstall.Json.error.code)" $testUninstall.Name
                    }
                }
                else {
                    Add-Result "test-apk" "install" "BLOCKED" "code=$($install.Json.error.code); HyperOS may require Install via USB" $install.Name
                }
            }
        }
    }
    else {
        Add-Result "test-apk" "install" "N/A" "no test APK requested; pre-existing apps untouched" ""
    }

    for ($cycle = 1; $cycle -le [Math]::Max(0, $SoakCycles); $cycle++) {
        $begin = Get-Date
        $cycleLaunch = Invoke-AfJson @("app", "launch", "com.android.settings", "--project", $projectDir, "--session", $sessionId, "--json")
        $cycleObserve = Invoke-AfJson @("ui", "observe", "--project", $projectDir, "--session", $sessionId, "--json")
        $cycleShot = Invoke-AfJson @("capture", "screenshot", "--project", $projectDir, "--session", $sessionId, "--label", "soak-$cycle", "--json")
        $cycleHome = Invoke-AfJson @("ui", "home", "--project", $projectDir, "--session", $sessionId, "--json")
        $ok = $cycleLaunch.Json.status -eq "success" -and $cycleObserve.Json.status -eq "success" -and $cycleShot.Json.status -eq "success" -and $cycleHome.Json.status -eq "success"
        Add-Result "soak" "cycle-$cycle" $(if ($ok) { "PASS" } else { "FAIL" }) ("elapsed={0:N0}ms" -f ((Get-Date) - $begin).TotalMilliseconds) "$($cycleObserve.Name), $($cycleShot.Name)"
        $health2 = Invoke-AfJson @("session", "status", "--project", $projectDir, "--session", $sessionId, "--json")
        if ($health2.Json.status -eq "success") {
            Add-Result "soak" "health-$cycle" "PASS" "state=$($health2.Json.result.state) heartbeat=$($health2.Json.result.heartbeat_at)" $health2.Name
        }
    }

    $finalProps = [ordered]@{}
    foreach ($name in @("ro.product.model", "ro.build.version.release", "ro.build.fingerprint")) {
        $finalProps[$name] = Get-Prop $name
    }
    $finalProps | ConvertTo-Json | Set-Content -Path (Join-Path $EvidenceDir "device-properties-final.json") -Encoding utf8
    $unchanged = $finalProps["ro.product.model"] -eq $props["ro.product.model"] -and $finalProps["ro.build.fingerprint"] -eq $props["ro.build.fingerprint"]
    Add-Result "cleanup" "device-unchanged" $(if ($unchanged) { "PASS" } else { "FAIL" }) "model and build fingerprint checked" "device-properties-final.json"
}
catch {
    Add-Result "harness" "exception" "FAIL" $_.Exception.Message ""
}
finally {
    if ($sessionId) {
        $release = Invoke-AfRaw @("session", "release", "--project", $projectDir, "--session", $sessionId, "--owner", $Owner, "--json")
        if ($release.ExitCode -eq 0) {
            Add-Result "cleanup" "session-release" "PASS" "released $sessionId" $release.Name
        }
        else {
            Add-Result "cleanup" "session-release" "FAIL" "exit=$($release.ExitCode)" $release.Name
        }
    }
    $online = & $adb devices -l 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "adb-devices-final.txt") -Value $online -Encoding utf8
    $stillOnline = $online -match ([regex]::Escape($Serial) + "\s+device")
    Add-Result "cleanup" "device-online" $(if ($stillOnline) { "PASS" } else { "FAIL" }) "final adb state" "adb-devices-final.txt"

    $summary = [pscustomobject]@{
        serial = $Serial
        evidence_dir = $EvidenceDir
        timestamp = (Get-Date -Format "o")
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        blocked = @($results | Where-Object { $_.status -eq "BLOCKED" }).Count
        na = @($results | Where-Object { $_.status -eq "N/A" }).Count
        results = $results
        non_destruction_statement = "No factory reset, no pm clear, no uninstall or clear-data of pre-existing apps, no data restore, no snapshot load, no security/lock changes, no network or time mutations were performed on the physical device."
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $EvidenceDir "real-device-summary.json") -Encoding utf8
    Set-Content -Path (Join-Path $EvidenceDir "TIMELINE.txt") -Value $timeline -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 4)
    if ($summary.fail -gt 0) { exit 1 }
}
