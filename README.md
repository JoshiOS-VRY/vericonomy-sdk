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
| `vericonomy-storage` | Keystore / UTXO / tx cache traits + `LightKeystoreService` |
| `vericonomy-storage-sqlite` | SQLite UTXO + history cache |
| `vericonomy-storage-file` | File keystore + secret envelope |
| `vericonomy-storage-ios` | iOS store bundle (Application Support + Keychain helpers) |
| `vericonomy-wallet-facade` | **Light-wallet orchestration** — gap scan, sync, session, send |
| `vericonomy-chain` | `ChainBackend` trait, Electrum + full-node RPC adapters |
| `vericonomy-ffi` | UniFFI `LightWalletSessionHandle` for iOS |
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

```bash
bash vericonomy-sdk/scripts/build-ios-ffi.sh
```

Or on Windows (host dylib only):

```powershell
powershell -File vericonomy-sdk/scripts/generate-swift-bindings.ps1
```

Outputs to `ios/VericonomyWallet/Generated/VericonomyFfi/`. The Swift app links `libvericonomy_ffi.a` built for `aarch64-apple-ios` / simulator.

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

- **Tauri desktop**: `vericonomy-wallet/desktop/verium-app/src-tauri` (full-node + light; path deps to this workspace)
- **iOS (native)**: SwiftUI + UniFFI `LightWalletSessionHandle` in `ios/VericonomyWallet/` — **preferred iOS path**
- **Tauri iOS**: deprecated after native app QA; see `ios/VericonomyWallet/docs/ios-native-cutover.md`
- **Qt full-node wallet**: `legacy/veribase` — future SDK integration via FFI/C++
