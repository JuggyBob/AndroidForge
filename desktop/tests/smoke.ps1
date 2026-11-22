#Requires -Version 5.1
<#
.SYNOPSIS
  Launch the AndroidForge desktop on every page, capture a screenshot, and
  record launch/render/UI-Automation evidence.

.DESCRIPTION
  This is a local release gate for the egui cockpit. It is intentionally
  conservative: a page that launches and renders is a PASS; a page whose
  interactive controls cannot be discovered through Windows UI Automation is
  reported as a limited-coverage warning, not a fabricated interaction pass.

  The harness never edits project state. It uses an isolated ANDROIDFORGE_HOME
  and only starts/stops the desktop processes it created.
#>
param(
    [string]$DesktopPath = "",
    [string]$CliPath = "",
    [string]$EvidenceDir = "",
    [int]$PageTimeoutSeconds = 20,
    [int]$SettleMilliseconds = 900,
    [switch]$UseExistingHome
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $DesktopPath) {
    $candidates = @(
        (Join-Path $repoRoot "target\release\androidforge-desktop.exe"),
        (Join-Path $repoRoot "target\debug\androidforge-desktop.exe")
    )
    $DesktopPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $CliPath) {
    $candidates = @(
        (Join-Path $repoRoot "target\release\androidforge.exe"),
        (Join-Path $repoRoot "target\debug\androidforge.exe")
    )
    $CliPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $DesktopPath -or -not (Test-Path $DesktopPath)) {
    throw "androidforge-desktop.exe was not found. Build the workspace or pass -DesktopPath."
}
if (-not $CliPath -or -not (Test-Path $CliPath)) {
    throw "androidforge.exe was not found. Build the workspace or pass -CliPath."
}
if (-not $EvidenceDir) {
    $EvidenceDir = Join-Path ([System.IO.Path]::GetTempPath()) ("androidforge-desktop-smoke-" + [guid]::NewGuid().ToString("N"))
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null

$desktopVersionOutput = (& $DesktopPath --version | Out-String).Trim()
if ($desktopVersionOutput -notmatch "androidforge-desktop\s+(\S+)") {
    throw "Could not read the desktop version from '$desktopVersionOutput'."
}
$desktopVersion = $Matches[1]
$cliVersionOutput = (& $CliPath --version | Out-String).Trim()
if ($cliVersionOutput -notmatch "androidforge\s+(\S+)") {
    throw "Could not read the CLI version from '$cliVersionOutput'."
}
$cliVersion = $Matches[1]
if ($desktopVersion -ne $cliVersion) {
    throw "Desktop version $desktopVersion does not match CLI version $cliVersion."
}

$pages = @(
    "setup",
    "tools",
    "overview",
    "context",
    "devices",
    "sessions",
    "matrix",
    "ownership",
    "runs",
    "triage",
    "compare",
    "logcat",
    "inspector",
    "maintenance",
    "settings"
)

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class AndroidForgeSmokeWin32 {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    public const int SW_RESTORE = 9;
}
"@

function Get-SmokeWindow {
    param([int]$ProcessId)
    $script:SmokeWindows = @()
    $callback = [AndroidForgeSmokeWin32+EnumWindowsProc]{
        param([IntPtr]$handle, [IntPtr]$param)
        [uint32]$owner = 0
        [AndroidForgeSmokeWin32]::GetWindowThreadProcessId($handle, [ref]$owner) | Out-Null
        if ($owner -eq $ProcessId) {
            $text = New-Object System.Text.StringBuilder 512
            [AndroidForgeSmokeWin32]::GetWindowText($handle, $text, 512) | Out-Null
            $rect = New-Object AndroidForgeSmokeWin32+RECT
            [AndroidForgeSmokeWin32]::GetWindowRect($handle, [ref]$rect) | Out-Null
            $script:SmokeWindows += [pscustomobject]@{
                Handle = $handle
                Title = $text.ToString()
                Visible = [AndroidForgeSmokeWin32]::IsWindowVisible($handle)
                Width = ($rect.Right - $rect.Left)
                Height = ($rect.Bottom - $rect.Top)
            }
        }
        return $true
    }
    [AndroidForgeSmokeWin32]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
    # `MainWindowHandle` can point at a hidden 18x18 helper, and the process
    # also owns a console window whose title is the executable path. Only the
    # real cockpit window has the exact "AndroidForge" title; do not fall back
    # to an untitled/largest window or the next capture will race the real one.
    $candidates = @($script:SmokeWindows |
        Where-Object { $_.Title -eq "AndroidForge" -and $_.Width -ge 600 -and $_.Height -ge 400 } |
        Sort-Object { $_.Width * $_.Height } -Descending)
    if ($candidates.Count -gt 0) {
        return $candidates[0]
    }
    return $null
}

function Save-WindowScreenshot {
    param([IntPtr]$Handle, [string]$Path)
    $rect = New-Object AndroidForgeSmokeWin32+RECT
    if (-not [AndroidForgeSmokeWin32]::GetWindowRect($Handle, [ref]$rect)) {
        return $false
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) {
        return $false
    }
    [AndroidForgeSmokeWin32]::SetForegroundWindow($Handle) | Out-Null
    Start-Sleep -Milliseconds 250
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    $captured = [AndroidForgeSmokeWin32]::PrintWindow($Handle, $hdc, 2)
    $graphics.ReleaseHdc($hdc)
    if (-not $captured) {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    }
    $graphics.Dispose()
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    return $true
}

function Get-RenderScore {
    param([string]$Path)
    $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $colors = @{}
        $stepX = [Math]::Max(1, [int]($bitmap.Width / 20))
        $stepY = [Math]::Max(1, [int]($bitmap.Height / 20))
        for ($x = 0; $x -lt $bitmap.Width; $x += $stepX) {
            for ($y = 0; $y -lt $bitmap.Height; $y += $stepY) {
                $colors[$bitmap.GetPixel($x, $y).ToArgb()] = $true
            }
        }
        return $colors.Count
    }
    finally {
        $bitmap.Dispose()
    }
}

function Get-AutomationSummary {
    param([IntPtr]$Handle)
    try {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
        $elements = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )
        $items = @()
        foreach ($element in $elements) {
            if ($items.Count -ge 20) { break }
            try {
                $items += [pscustomobject]@{
                    name = $element.Current.Name
                    control_type = $element.Current.ControlType.ProgrammaticName
                    enabled = $element.Current.IsEnabled
                }
            } catch { }
        }
        return [pscustomobject]@{
            available = $true
            count = $elements.Count
            sample = $items
        }
    }
    catch {
        return [pscustomobject]@{
            available = $false
            count = 0
            sample = @()
            reason = $_.Exception.Message
        }
    }
}

function Invoke-AutomationButton {
    param([IntPtr]$Handle, [string]$NamePattern)
    try {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
        $buttons = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::Button
            )
        )
        foreach ($button in $buttons) {
            try {
                if ($button.Current.Name -match $NamePattern -and $button.Current.IsEnabled) {
                    $pattern = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
                    $pattern.Invoke()
                    return [pscustomobject]@{
                        invoked = $true
                        name = $button.Current.Name
                    }
                }
            }
            catch { }
        }
        return [pscustomobject]@{
            invoked = $false
            name = $null
            reason = "no enabled button matched '$NamePattern'"
        }
    }
    catch {
        return [pscustomobject]@{
            invoked = $false
            name = $null
            reason = $_.Exception.Message
        }
    }
}

$oldHome = $env:ANDROIDFORGE_HOME
$oldExe = $env:ANDROIDFORGE_EXE
$env:ANDROIDFORGE_EXE = $CliPath
if (-not $UseExistingHome) {
    $env:ANDROIDFORGE_HOME = Join-Path $EvidenceDir "home"
    New-Item -ItemType Directory -Force -Path $env:ANDROIDFORGE_HOME | Out-Null
}

$results = @()
try {
    foreach ($page in $pages) {
        $started = Get-Date
        $screenshot = Join-Path $EvidenceDir ("page-{0}.png" -f $page)
        $process = $null
        $status = "PASS"
        $reason = ""
        $title = ""
        $automation = $null
        $interaction = $null
        $renderScore = 0
        try {
            $process = Start-Process -FilePath $DesktopPath -ArgumentList @("--page", $page) -PassThru
            $deadline = (Get-Date).AddSeconds($PageTimeoutSeconds)
            $window = $null
            while ((Get-Date) -lt $deadline -and -not $process.HasExited) {
                Start-Sleep -Milliseconds 250
                $process.Refresh()
                if (-not $process.HasExited) {
                    $window = Get-SmokeWindow -ProcessId $process.Id
                }
                if ($window -and $window.Title -eq "AndroidForge") {
                    break
                }
            }
            if ($process.HasExited) {
                $status = "FAIL"
                $reason = "process exited before a window appeared (exit $($process.ExitCode))"
            }
            elseif (-not $window) {
                $status = "FAIL"
                $reason = "no cockpit window appeared within $PageTimeoutSeconds seconds"
            }
            else {
                # A window handle can exist before the first frame is painted.
                # Restore it, wait for the UIA tree to populate, then retry blank
                # captures. A title bar plus scrollbar alone is not a rendered page.
                [AndroidForgeSmokeWin32]::ShowWindow($window.Handle, [AndroidForgeSmokeWin32]::SW_RESTORE) | Out-Null
                Start-Sleep -Milliseconds 350
                $process.Refresh()
                if ($window.Title) {
                    $title = $window.Title
                }
                $automation = Get-AutomationSummary -Handle $window.Handle
                $uiDeadline = (Get-Date).AddSeconds(8)
                while ((Get-Date) -lt $uiDeadline -and -not $process.HasExited -and $automation.available -and $automation.count -lt 8) {
                    Start-Sleep -Milliseconds 300
                    $process.Refresh()
                    $automation = Get-AutomationSummary -Handle $window.Handle
                }
                Start-Sleep -Milliseconds $SettleMilliseconds
                $process.Refresh()
                $captured = Save-WindowScreenshot -Handle $window.Handle -Path $screenshot
                $attempt = 1
                while ($captured -and (Test-Path $screenshot)) {
                    $size = (Get-Item $screenshot).Length
                    $renderScore = Get-RenderScore -Path $screenshot
                    $uiReady = (-not $automation.available) -or ($automation.count -ge 8)
                    if ($size -ge 20000 -and $renderScore -ge 6 -and $uiReady) {
                        break
                    }
                    if ($attempt -ge 5 -or $process.HasExited) {
                        break
                    }
                    $attempt += 1
                    Start-Sleep -Milliseconds 700
                    $process.Refresh()
                    if ($process.HasExited) {
                        break
                    }
                    $automation = Get-AutomationSummary -Handle $window.Handle
                    $captured = Save-WindowScreenshot -Handle $window.Handle -Path $screenshot
                }
                if (-not $captured -or -not (Test-Path $screenshot)) {
                    $status = "FAIL"
                    $reason = "window screenshot could not be captured"
                }
                else {
                    $size = (Get-Item $screenshot).Length
                    $renderScore = Get-RenderScore -Path $screenshot
                    if ($size -lt 20000 -or $renderScore -lt 6) {
                        $status = "FAIL"
                        $reason = "screenshot is blank or too small (bytes=$size colors=$renderScore)"
                    }
                    elseif ([string]::IsNullOrWhiteSpace($title)) {
                        $status = "FAIL"
                        $reason = "cockpit window has no title"
                    }
                    elseif ($automation.available -and $automation.count -lt 8) {
                        $status = "FAIL"
                        $reason = "UI Automation tree did not populate (elements=$($automation.count))"
                    }
                }
            }
            if ($status -eq "PASS" -and $page -eq "overview") {
                $interaction = Invoke-AutomationButton -Handle $window.Handle -NamePattern "Refresh"
                Start-Sleep -Seconds 1
                $process.Refresh()
                if ($process.HasExited) {
                    $status = "FAIL"
                    $reason = "process exited after invoking the Refresh button"
                }
            }
        }
        catch {
            $status = "FAIL"
            $reason = $_.Exception.Message
        }
        finally {
            if ($process -and -not $process.HasExited) {
                $process.CloseMainWindow() | Out-Null
                if (-not $process.WaitForExit(5000)) {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                }
            }
        }
        $results += [pscustomobject]@{
            page = $page
            status = $status
            reason = $reason
            window_title = $title
            screenshot = $screenshot
            screenshot_bytes = if (Test-Path $screenshot) { (Get-Item $screenshot).Length } else { 0 }
            render_colors = $renderScore
            duration_ms = [int]((Get-Date) - $started).TotalMilliseconds
            ui_automation = $automation
            interaction = $interaction
        }
    }
}
finally {
    $env:ANDROIDFORGE_HOME = $oldHome
    $env:ANDROIDFORGE_EXE = $oldExe
}

$summary = [pscustomobject]@{
    desktop = $DesktopPath
    cli = $CliPath
    desktop_version = $desktopVersion
    cli_version = $cliVersion
    evidence_dir = $EvidenceDir
    pages = $results
    pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
    fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
    ui_automation_available = @($results | Where-Object { $_.ui_automation.available }).Count -gt 0
    interaction_coverage = if (@($results | Where-Object { $_.ui_automation.available }).Count -gt 0) {
        $refresh = @($results | Where-Object { $_.interaction.invoked }).Count
        "UIAutomation tree discovered; Refresh invoked on $refresh page(s). Other interactive actions are intentionally not automated by this smoke harness."
    } else {
        "BLOCKED: Windows UI Automation did not expose desktop controls. Page launch/render only."
    }
}
$summaryPath = Join-Path $EvidenceDir "desktop-smoke.json"
$summary | ConvertTo-Json -Depth 8 | Set-Content -Path $summaryPath -Encoding utf8

Write-Output ($summary | ConvertTo-Json -Depth 4)
if ($summary.fail -gt 0) {
    exit 1
}
