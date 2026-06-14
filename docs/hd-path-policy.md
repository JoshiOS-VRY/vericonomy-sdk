# HD path policy

Vericonomy wallets use two HD derivation schemes depending on how the seed was created.

## BIP44 light wallets (in-app mnemonic)

New light wallets created with a BIP39 mnemonic use standard BIP44 paths:

| Chain    | Path                      | SLIP-44 coin type |
|----------|---------------------------|-------------------|
| Verium   | `m/44'/462'/0'/0/n`       | 462               |
| Vericoin | `m/44'/463'/0'/0/n`       | 463               |

- Only the **external (receive)** chain is used (`…/0/n`).
- Change outputs are not derived on a separate internal chain for BIP44 mnemonics.
- Address index `n` is non-hardened.

## Full-node HD (wallet.dat / exported master key)

Wallets restored from `wallet.dat` or a Vericonomy extended master key (`xprv` or chain-specific Base58 prefix) use Bitcoin Core-style paths from `wallet.cpp`:

| Purpose | Path           |
|---------|----------------|
| Receive | `m/0'/0'/n'`    |
| Change  | `m/0'/1'/n'`    |

- Account `0'`, chain `0'` (external) or `1'` (internal), address index `n'` are all **hardened**.
- Gap scan and signing search both external and internal chains up to index 501.

## Detecting which scheme applies

`vericonomy_hd::is_hd_master_secret(coin, secret)` returns true for:

- Standard Bitcoin `xprv…` strings
- Vericonomy extended-secret exports (chain-specific 4-byte Base58 prefix + 74-byte payload)

Otherwise the seed is treated as a BIP39 mnemonic phrase.

## Address encoding

Both schemes produce legacy P2PKH addresses:

- Version byte: **70** (mainnet VRM/VRC)
- WIF secret prefix: **198** (`128 + 70`)

See `vericonomy-chain-params` for per-chain constants.
