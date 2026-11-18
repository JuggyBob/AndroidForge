#Requires -Version 5.1
<#
.SYNOPSIS
  Verify the AndroidForge MCP stdio server with a real JSON-RPC handshake.
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
    $EvidenceDir = Join-Path $repoRoot "target\mcp-smoke\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$results = New-Object System.Collections.Generic.List[object]

function Add-Result([string]$Name, [bool]$Ok, [string]$Detail) {
    $status = if ($Ok) { "PASS" } else { "FAIL" }
    $results.Add([pscustomobject]@{ name = $Name; status = $status; detail = $Detail })
    Write-Output ("{0} {1}: {2}" -f $status, $Name, $Detail)
}

$startInfo = New-Object System.Diagnostics.ProcessStartInfo
$startInfo.FileName = $CliPath
$startInfo.Arguments = "mcp serve"
$startInfo.UseShellExecute = $false
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.CreateNoWindow = $true
$process = New-Object System.Diagnostics.Process
$process.StartInfo = $startInfo

try {
    [void]$process.Start()
    $stdin = $process.StandardInput
    $stdin.AutoFlush = $true
    $stdout = $process.StandardOutput
    $stderr = $process.StandardError

    function Send-Mcp([hashtable]$Message) {
        $json = $Message | ConvertTo-Json -Depth 20 -Compress
        Set-Content -Path (Join-Path $EvidenceDir "request-$($Message.id).json") -Value $json -Encoding utf8
        $stdin.WriteLine($json)
        $line = $stdout.ReadLine()
        if (-not $line) {
            $err = $stderr.ReadToEnd()
            throw "MCP stdout ended. stderr: $err"
        }
        Set-Content -Path (Join-Path $EvidenceDir "response-$($Message.id).json") -Value $line -Encoding utf8
        return $line | ConvertFrom-Json
    }

    $init = Send-Mcp @{
        jsonrpc = "2.0"
        id = 1
        method = "initialize"
        params = @{
            protocolVersion = "2025-06-18"
            capabilities = @{}
            clientInfo = @{ name = "androidforge-mcp-smoke"; version = "1" }
        }
    }
    $initOk = $init.result.serverInfo.name -eq "androidforge" -and $init.result.capabilities.tools
    Add-Result "initialize" $initOk "server=$($init.result.serverInfo.name) version=$($init.result.serverInfo.version) protocol=$($init.result.protocolVersion)"

    $stdin.WriteLine((@{ jsonrpc = "2.0"; method = "notifications/initialized"; params = @{} } | ConvertTo-Json -Compress))

    $ping = Send-Mcp @{ jsonrpc = "2.0"; id = 2; method = "ping"; params = @{} }
    Add-Result "ping" ($null -ne $ping.result) "result present"

    $list = Send-Mcp @{ jsonrpc = "2.0"; id = 3; method = "tools/list"; params = @{} }
    $toolNames = @($list.result.tools | ForEach-Object { $_.name })
    $required = @(
        "androidforge_context",
        "androidforge_ui_tap",
        "androidforge_app_uninstall",
        "androidforge_app_launch",
        "androidforge_app_broadcast",
        "androidforge_diagnostics_logcat_query",
        "androidforge_diagnostics_logcat_watch",
        "androidforge_network_proxy_set",
        "androidforge_network_profile_apply",
        "androidforge_network_restore",
        "androidforge_run_checkpoint",
        "androidforge_run_resume",
        "androidforge_run_events",
        "androidforge_report_repro"
    )
    $missing = @($required | Where-Object { $toolNames -notcontains $_ })
    $listOk = $toolNames.Count -ge 31 -and $missing.Count -eq 0
    Add-Result "tools/list" $listOk "count=$($toolNames.Count) missing=$($missing -join ',')"

    $version = Send-Mcp @{
        jsonrpc = "2.0"; id = 4; method = "tools/call"
        params = @{ name = "androidforge_version"; arguments = @{} }
    }
    $versionText = ($version.result.content | Where-Object { $_.type -eq "text" } | Select-Object -First 1).text
    Add-Result "tools/call version" ($versionText -match "androidforge 0\.10\.0") $versionText

    $doctor = Send-Mcp @{
        jsonrpc = "2.0"; id = 5; method = "tools/call"
        params = @{ name = "androidforge_doctor"; arguments = @{} }
    }
    $doctorOk = $doctor.result.structuredContent.status -eq "success"
    Add-Result "tools/call doctor" $doctorOk "status=$($doctor.result.structuredContent.status) isError=$($doctor.result.isError)"

    $profiles = Send-Mcp @{
        jsonrpc = "2.0"; id = 9; method = "tools/call"
        params = @{ name = "androidforge_network_profile_list"; arguments = @{} }
    }
    $profileNames = @($profiles.result.structuredContent.result.profiles | ForEach-Object { $_.name })
    $profilesOk = $profileNames.Count -eq 5 -and $profileNames -contains "wifi-only"
    Add-Result "tools/call network profiles" $profilesOk "count=$($profileNames.Count) names=$($profileNames -join ',')"

    $resources = Send-Mcp @{ jsonrpc = "2.0"; id = 6; method = "resources/list"; params = @{} }
    $uris = @($resources.result.resources | ForEach-Object { $_.uri })
    Add-Result "resources/list" ($uris -contains "androidforge://tools" -and $uris -contains "androidforge://schemas") "count=$($uris.Count)"

    $read = Send-Mcp @{
        jsonrpc = "2.0"; id = 7; method = "resources/read"
        params = @{ uri = "androidforge://schema/observation.schema.json" }
    }
    $text = ($read.result.contents | Select-Object -First 1).text
    $schemaOk = $false
    try {
        $schema = $text | ConvertFrom-Json
        $schemaOk = $schema.title -like "*observation*"
    } catch { }
    Add-Result "resources/read schema" $schemaOk "bytes=$($text.Length)"

    $shutdown = Send-Mcp @{ jsonrpc = "2.0"; id = 8; method = "shutdown"; params = @{} }
    Add-Result "shutdown" ($null -ne $shutdown.result) "result present"
    $stdin.Close()
    if (-not $process.WaitForExit(10000)) {
        $process.Kill()
        Add-Result "process-exit" $false "server did not exit within 10s"
    }
    else {
        Add-Result "process-exit" ($process.ExitCode -eq 0) "exit=$($process.ExitCode)"
    }
}
catch {
    Add-Result "harness" $false $_.Exception.Message
}
finally {
    if (-not $process.HasExited) { try { $process.Kill() } catch { } }
    $summary = [pscustomobject]@{
        evidence_dir = $EvidenceDir
        cli = $CliPath
        timestamp = (Get-Date -Format "o")
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        results = $results
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $EvidenceDir "mcp-summary.json") -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 3)
    if ($summary.fail -gt 0) { exit 1 }
}

