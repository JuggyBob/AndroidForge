#Requires -Version 5.1
<#
.SYNOPSIS
  Local supply-chain inventory and policy checks.

.DESCRIPTION
  Records package/license inventory, duplicate versions, wildcard dependency
  checks and optional cargo-audit results. It never modifies Cargo.lock.
#>
param(
    [string]$EvidenceDir = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $EvidenceDir) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $EvidenceDir = Join-Path $repoRoot "target\supply-chain\$stamp"
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$results = New-Object System.Collections.Generic.List[object]

function Add-Result([string]$Name, [string]$Status, [string]$Detail, [string]$Evidence) {
    $results.Add([pscustomobject]@{ name = $Name; status = $Status; detail = $Detail; evidence = $Evidence })
    Write-Output ("{0} {1}: {2} ({3})" -f $Status, $Name, $Detail, $Evidence)
}

try {
    if (-not (Test-Path (Join-Path $repoRoot "Cargo.lock"))) {
        throw "Cargo.lock is missing."
    }
    Add-Result "cargo-lock" "PASS" "Cargo.lock present" ""

    $deny = Join-Path $repoRoot "deny.toml"
    Add-Result "deny-config" $(if (Test-Path $deny) { "PASS" } else { "FAIL" }) "deny.toml policy" $deny

    $metadataRaw = cargo metadata --format-version 1 --locked 2>&1 | Out-String
    $metadata = $metadataRaw | ConvertFrom-Json
    $packages = $metadata.packages
    $inventory = $packages | ForEach-Object {
        [pscustomobject]@{
            name = $_.name
            version = $_.version
            license = $_.license
            license_file = $_.license_file
            repository = $_.repository
            source = $_.source
        }
    } | Sort-Object name, version
    $inventory | ConvertTo-Json -Depth 4 | Set-Content -Path (Join-Path $EvidenceDir "packages.json") -Encoding utf8
    $unlicensed = @($inventory | Where-Object { -not $_.license -and -not $_.license_file })
    Add-Result "licenses" $(if ($unlicensed.Count -eq 0) { "PASS" } else { "FAIL" }) "$($inventory.Count) packages; unlicensed=$($unlicensed.Count)" "packages.json"
    if ($unlicensed.Count -gt 0) {
        $unlicensed | ConvertTo-Json -Depth 3 | Set-Content -Path (Join-Path $EvidenceDir "unlicensed.json") -Encoding utf8
    }

    $tree = cargo tree --duplicates 2>&1 | Out-String
    Set-Content -Path (Join-Path $EvidenceDir "duplicates.txt") -Value $tree -Encoding utf8
    $duplicateLines = @($tree -split "`r?`n" | Where-Object { $_ -match "^[A-Za-z0-9_-]+ v" })
    Add-Result "duplicates" $(if ($duplicateLines.Count -eq 0) { "PASS" } else { "WARN" }) "$($duplicateLines.Count) duplicate-version groups" "duplicates.txt"

    $manifest = Get-Content (Join-Path $repoRoot "Cargo.toml") -Raw
    $wildcard = $manifest -match '\*\s*"'
    Add-Result "wildcards" $(if (-not $wildcard) { "PASS" } else { "FAIL" }) "workspace manifest wildcard scan" ""

    $audit = Get-Command cargo-audit -ErrorAction SilentlyContinue
    if ($audit) {
        $auditOut = cargo audit 2>&1 | Out-String
        $auditCode = $LASTEXITCODE
        Set-Content -Path (Join-Path $EvidenceDir "cargo-audit.txt") -Value $auditOut -Encoding utf8
        Add-Result "cargo-audit" $(if ($auditCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$auditCode" "cargo-audit.txt"
    }
    else {
        Add-Result "cargo-audit" "BLOCKED" "cargo-audit is not installed; CI runs cargo-deny advisories" ""
    }

    $denyCommand = Get-Command cargo-deny -ErrorAction SilentlyContinue
    if ($denyCommand) {
        $denyOut = cargo deny check advisories bans licenses sources 2>&1 | Out-String
        $denyCode = $LASTEXITCODE
        Set-Content -Path (Join-Path $EvidenceDir "cargo-deny.txt") -Value $denyOut -Encoding utf8
        Add-Result "cargo-deny" $(if ($denyCode -eq 0) { "PASS" } else { "FAIL" }) "exit=$denyCode" "cargo-deny.txt"
    }
    else {
        Add-Result "cargo-deny" "BLOCKED" "cargo-deny is not installed locally; CI runs the same policy" ""
    }
}
catch {
    Add-Result "harness" "FAIL" $_.Exception.Message ""
}
finally {
    $summary = [pscustomobject]@{
        evidence_dir = $EvidenceDir
        timestamp = (Get-Date -Format "o")
        pass = @($results | Where-Object { $_.status -eq "PASS" }).Count
        warn = @($results | Where-Object { $_.status -eq "WARN" }).Count
        fail = @($results | Where-Object { $_.status -eq "FAIL" }).Count
        blocked = @($results | Where-Object { $_.status -eq "BLOCKED" }).Count
        results = $results
    }
    $summary | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $EvidenceDir "supply-chain-summary.json") -Encoding utf8
    Write-Output ($summary | ConvertTo-Json -Depth 3)
    if ($summary.fail -gt 0) { exit 1 }
}
