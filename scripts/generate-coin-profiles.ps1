# Generate coin-profiles.json for the Verium desktop app from vericonomy-sdk chain params.
param(
    [string]$SdkRoot = (Join-Path $PSScriptRoot ".."),
    [string]$OutPath = (Join-Path $PSScriptRoot "..\..\verium\desktop\verium-app\src\lib\coin\coin-profiles.json")
)

$ErrorActionPreference = "Stop"

$cliDir = Join-Path $SdkRoot "vericonomy-cli"
if (-not (Test-Path $cliDir)) {
    throw "vericonomy-cli not found at $cliDir"
}

$json = & cargo run --quiet --manifest-path (Join-Path $cliDir "Cargo.toml") -- profiles-json 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "profiles-json failed: $json"
}

$outDir = Split-Path -Parent $OutPath
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

Set-Content -Path $OutPath -Value $json -Encoding UTF8
Write-Host "Wrote $OutPath"
