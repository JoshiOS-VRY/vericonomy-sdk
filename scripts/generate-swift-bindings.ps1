# Generate Swift UniFFI bindings from vericonomy-ffi.
# Requires: cargo build -p vericonomy-ffi, uniffi-bindgen-cli 0.28 on PATH.

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$SdkRoot = Join-Path $Root "vericonomy-sdk"
$FfiCrate = Join-Path $SdkRoot "vericonomy-ffi"
$OutDir = Join-Path $Root "ios\VericonomyWallet\Generated\VericonomyFfi"

Write-Host "Building vericonomy-ffi..."
Push-Location $SdkRoot
cargo build -p vericonomy-ffi
if ($LASTEXITCODE -ne 0) { Pop-Location; exit $LASTEXITCODE }
Pop-Location

$LibDir = Join-Path $SdkRoot "target\debug"
$LibName = if ($IsWindows -or $env:OS -eq "Windows_NT") { "vericonomy_ffi.dll" } else { "libvericonomy_ffi.so" }
$LibPath = Join-Path $LibDir $LibName

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

if (Test-Path $LibPath) {
    Write-Host "Generating Swift bindings to $OutDir"
    Push-Location $SdkRoot
    cargo run -p vericonomy-ffi --bin uniffi-bindgen -- generate `
        --library $LibPath `
        --language swift `
        --out-dir $OutDir
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { exit $code }
    Write-Host "Done. Link $LibPath (or release / iOS static build) in Xcode."
} elseif (Get-Command uniffi-bindgen -ErrorAction SilentlyContinue) {
    Write-Host "Generating Swift bindings to $OutDir"
    uniffi-bindgen generate `
        --library $LibPath `
        --language swift `
        --out-dir $OutDir
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "Done. Link $LibPath (or release build) in Xcode."
} else {
    Write-Host "uniffi-bindgen not found. Install: cargo install uniffi-bindgen-cli --version 0.28"
    Write-Host "Scaffold Swift protocol at $OutDir\WalletFacadeProtocol.swift"
    @"
import Foundation

/// Placeholder until 'uniffi-bindgen' is run. Matches vericonomy-ffi WalletFacade API.
public protocol WalletFacadeProtocol {
    func validateMnemonic(_ phrase: String) throws -> Bool
    func generateMnemonic() throws -> String
    func deriveAddress(coin: String, mnemonic: String, index: UInt32) throws -> String
    func validateSendAddress(coin: String, address: String) throws
    func defaultElectrumServers(coin: String) throws -> [String]
    func getLightBalance(coin: String, mnemonic: String, maxIndex: UInt32) throws -> BalanceInfoStub
}

public struct BalanceInfoStub {
    public let confirmedSats: Int64
    public let unconfirmedSats: Int64
    public let totalSats: Int64
}

public final class WalletFacadeStub: WalletFacadeProtocol {
    public init() {}
    public func validateMnemonic(_ phrase: String) throws -> Bool { false }
    public func generateMnemonic() throws -> String {
        throw NSError(domain: "VericonomyFfi", code: -1, userInfo: [NSLocalizedDescriptionKey: "Run generate-swift-bindings.ps1"])
    }
    public func deriveAddress(coin: String, mnemonic: String, index: UInt32) throws -> String {
        throw NSError(domain: "VericonomyFfi", code: -1, userInfo: [NSLocalizedDescriptionKey: "Run generate-swift-bindings.ps1"])
    }
    public func validateSendAddress(coin: String, address: String) throws {}
    public func defaultElectrumServers(coin: String) throws -> [String] { [] }
    public func getLightBalance(coin: String, mnemonic: String, maxIndex: UInt32) throws -> BalanceInfoStub {
        throw NSError(domain: "VericonomyFfi", code: -1, userInfo: [NSLocalizedDescriptionKey: "Run generate-swift-bindings.ps1"])
    }
}
"@ | Set-Content -Encoding UTF8 (Join-Path $OutDir "WalletFacadeProtocol.swift")
}
