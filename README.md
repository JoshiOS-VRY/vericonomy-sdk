# Vericonomy SDK (`vericonomy-sdk/`)

Shared Rust wallet engine for Vericonomy light-wallet clients (Tauri desktop, iOS, CLI).

## Crates

| Crate | Role |
|-------|------|
| `vericonomy-errors` | Stable error types for FFI |
| `vericonomy-chain-params` | VRC/VRM chain constants + `coin-profiles.json` export |
| `vericonomy-wallet-core` | BIP39, BIP32, WIF |
| `vericonomy-tx` | nTime wire format, sighash, signing primitives |
| `vericonomy-hd` | HD derivation, P2PKH addresses |
| `vericonomy-wallet-engine` | UTXO selection, send planning, local signing |
| `vericonomy-chain` | `ChainBackend` trait, Electrum + full-node RPC adapters |
| `vericonomy-storage` | `SecretStore`, `Keystore`, `TxCache` traits |
| `vericonomy-ffi` | UniFFI surface for iOS (Android deferred) |
| `vericonomy-cli` | Headless wallet tool |

## Build & test

```bash
cd vericonomy-sdk
cargo test --workspace
```

## Generate coin profiles (for TypeScript shells)

```powershell
powershell -File scripts/generate-coin-profiles.ps1
```

## Generate iOS Swift bindings (UniFFI)

```powershell
powershell -File scripts/generate-swift-bindings.ps1
```

Uses `cargo run -p vericonomy-ffi --bin uniffi-bindgen` (no global install required).
Outputs to `ios/VericonomyWallet/Generated/VericonomyFfi/`.

## CLI

```bash
cargo run -p vericonomy-cli -- mnemonic validate "word1 word2 ..."
cargo run -p vericonomy-cli -- address derive verium "word1 ..." 0
cargo run -p vericonomy-cli -- profiles-json
```

## Docs

- [HD path policy](docs/hd-path-policy.md)
- [wallet.dat migration](docs/wallet-dat-migration.md)

## Consumers

- **Tauri desktop**: `verium/desktop/verium-app/src-tauri` (path deps to this workspace)
- **iOS**: `ios/VericonomyWallet/` (SwiftUI + UniFFI bindings)
- **Qt full-node wallet**: `legacy/veribase` — future SDK integration via FFI/C++
