# wallet.dat migration

This document describes how full-node `wallet.dat` keys relate to the portable SDK and light-wallet flows.

## What full-node wallets store

`veriumd` / `vericoind` persist an encrypted Berkeley DB wallet file (`wallet.dat`) containing:

- HD master key (Core-style `m/0'/…` derivation)
- Encrypted private keys for mined/staked UTXOs
- Transaction labels and metadata

The SDK does **not** parse `wallet.dat` directly. Desktop shells use RPC (`dumpwallet`, `exportmasterkey`, etc.) or file copy plus daemon unlock.

## Migration paths

### Stay on full node

Keep `wallet.dat` in the coin datadir. The app uses `vericonomy-chain::FullNodeRpcClient` with a shell-provided `JsonRpcClient`. No seed export required.

### Move to light wallet (Electrum)

1. Export the HD master key from the running node (Security → Export, or RPC where available).
2. Import the master key string into the light wallet as the seed secret.
3. The SDK detects a master key via `is_hd_master_secret` and uses `m/0'/0'/n'` / `m/0'/1'/n'` paths.
4. Run an initial gap scan against Electrum servers to discover funded addresses.

### New BIP39 wallet

Creating a fresh mnemonic in the app uses BIP44 paths (`m/44'/coin'/0'/0/n`). This is **not** compatible with existing `wallet.dat` address sequences. Users should not mix a new mnemonic with an old `wallet.dat` on the same profile.

## WIF and sethdseed

When importing a recovery phrase into a full node via `sethdseed`, the WIF must use secret prefix **198**. The SDK encodes WIF through `vericonomy_wallet_core::secret_bytes_to_wif` with the chain profile's `wif_secret_prefix`.

## Recommended checks before migration

1. Confirm backup of `wallet.dat` and any wallet passphrase.
2. Compare a derived address at index 0 against a known receive address from the full node.
3. For light mode, verify Electrum server connectivity (`vericonomy-chain` conformance helpers).

## Storage in mobile shells

Encrypted mnemonics and passphrases should use platform `SecretStore` / `Keystore` implementations from `vericonomy-storage`, not plaintext files.
