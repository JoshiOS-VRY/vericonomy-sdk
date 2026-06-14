//! HD derivation and P2PKH address encoding for light wallet.
//!
//! Full-node Verium/Vericoin wallets use Bitcoin Core-style paths (`m/0'/0'/n'` receive,
//! `m/0'/1'/n'` change). BIP39 light wallets created in-app use BIP44 (`m/44'/coin'/0'/0/n`).

use std::str::FromStr;

use bip39::{Language, Mnemonic};
use bitcoin::bip32::{ChainCode, ChildNumber, DerivationPath, Fingerprint, Xpriv};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{PrivateKey, PublicKey, PubkeyHash, ScriptBuf};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_wallet_core::secret_bytes_to_wif;

const COIN_SATS: f64 = 100_000_000.0;
const BIP32_EXTKEY_SIZE: usize = 74;
const EXT_SECRET_PREFIX_LEN: usize = 4;

/// Maximum HD index scanned when resolving signing keys (matches desktop gap scan).
pub const GAP_SCAN_MAX_INDEX: u32 = 501;

/// External (receive) or internal (change) chain — matches `wallet.cpp` HD layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdChain {
    External,
    Internal,
}

impl HdChain {
    fn hardened_index(self) -> u32 {
        match self {
            HdChain::External => 0,
            HdChain::Internal => 1,
        }
    }
}

/// Full-node HD master keys use Core-style paths; BIP39 mnemonics use BIP44.
pub fn uses_core_hd_paths(coin: CoinId, seed_secret: &str) -> bool {
    is_hd_master_secret(coin, seed_secret)
}

pub fn coin_type_for(coin: CoinId) -> u32 {
    coin.profile().bip44_coin_type
}

pub fn secret_key_prefix(coin: CoinId) -> u8 {
    coin.profile().wif_secret_prefix
}

pub fn pubkey_address_version(coin: CoinId) -> u8 {
    coin.profile().p2pkh_version
}

pub fn sats_to_coins(sats: i64) -> f64 {
    sats as f64 / COIN_SATS
}

pub fn coins_to_sats(coins: f64) -> i64 {
    (coins * COIN_SATS).round() as i64
}

fn base58check_encode(version: u8, payload: &[u8]) -> String {
    let mut data = Vec::with_capacity(1 + payload.len() + 4);
    data.push(version);
    data.extend_from_slice(payload);
    let hash1 = Sha256::digest(&data);
    let hash2 = Sha256::digest(hash1);
    data.extend_from_slice(&hash2[..4]);
    bs58::encode(data).into_string()
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let rip = Ripemd160::digest(sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&rip);
    out
}

pub fn pubkey_to_p2pkh_address(coin: CoinId, pubkey: &[u8]) -> Result<String> {
    let pk_hash = hash160(pubkey);
    Ok(base58check_encode(pubkey_address_version(coin), &pk_hash))
}

/// Decode a standard P2PKH `scriptPubKey` (`OP_DUP OP_HASH160 … OP_EQUALVERIFY OP_CHECKSIG`).
pub fn p2pkh_script_to_address(coin: CoinId, script: &[u8]) -> Option<String> {
    if script.len() != 25
        || script[0] != 0x76
        || script[1] != 0xa9
        || script[2] != 0x14
        || script[23] != 0x88
        || script[24] != 0xac
    {
        return None;
    }
    Some(base58check_encode(
        pubkey_address_version(coin),
        &script[3..23],
    ))
}

/// Known Vericonomy extended-secret Base58 prefixes (mainnet + test variants).
fn ext_secret_prefixes(coin: CoinId) -> &'static [[u8; EXT_SECRET_PREFIX_LEN]] {
    match coin {
        CoinId::Verium => &[
            [0xE3, 0xCC, 0xAE, 0x01], // mainnet
            [0x04, 0x35, 0x83, 0x94], // legacy test/regtest
            [0xDA, 0xCE, 0xCE, 0x01], // binarytest VRM
        ],
        CoinId::Vericoin => &[
            [0xE3, 0xCC, 0xAE, 0x01], // mainnet
            [0xDA, 0xCE, 0xAE, 0x01], // binarytest VRC
        ],
    }
}

pub fn normalize_hd_master_secret(secret: &str) -> String {
    secret
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn is_numbered_prefix_token(token: &str) -> bool {
    let t = token.trim_end_matches('.');
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// Collapse whitespace for BIP39 phrases; strips numbered-list prefixes from copy/paste.
pub fn normalize_mnemonic_phrase(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .filter(|w| !is_numbered_prefix_token(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize user-provided seed material: phrases keep spaced words; HD keys strip whitespace.
pub fn normalize_light_wallet_seed(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.starts_with("xprv") {
        return normalize_hd_master_secret(trimmed);
    }
    if trimmed.contains(char::is_whitespace) {
        normalize_mnemonic_phrase(trimmed)
    } else {
        normalize_hd_master_secret(trimmed)
    }
}

/// True when `secret` is a BIP32 master key (Bitcoin xprv or Vericonomy export).
pub fn is_hd_master_secret(coin: CoinId, secret: &str) -> bool {
    let trimmed = normalize_hd_master_secret(secret);
    if trimmed.starts_with("xprv") {
        return true;
    }
    if trimmed.contains(' ') {
        return false;
    }
    let Ok(data) = bs58::decode(&trimmed).with_check(None).into_vec() else {
        return false;
    };
    if data.len() != EXT_SECRET_PREFIX_LEN + BIP32_EXTKEY_SIZE {
        return false;
    }
    let prefix: [u8; EXT_SECRET_PREFIX_LEN] = data[0..EXT_SECRET_PREFIX_LEN]
        .try_into()
        .unwrap_or([0; 4]);
    ext_secret_prefixes(coin).contains(&prefix)
}

fn bip32_payload_to_xpriv(payload: &[u8]) -> Result<Xpriv> {
    if payload.len() != BIP32_EXTKEY_SIZE {
        return Err(WalletError::other("extended key payload has wrong length"));
    }
    if payload[41] != 0 {
        return Err(WalletError::other(
            "extended private key has invalid padding byte",
        ));
    }
    let depth = payload[0];
    let parent_fingerprint: [u8; 4] = payload[1..5]
        .try_into()
        .map_err(|_| WalletError::other("extended key fingerprint missing"))?;
    let parent_fingerprint = Fingerprint::from(parent_fingerprint);
    let child_number = u32::from_be_bytes([payload[5], payload[6], payload[7], payload[8]]);
    let child_number = ChildNumber::from(child_number);
    let chain_code_bytes: [u8; 32] = payload[9..41]
        .try_into()
        .map_err(|_| WalletError::other("extended key chain code missing"))?;
    let chain_code = ChainCode::from(chain_code_bytes);
    let private_key = bitcoin::secp256k1::SecretKey::from_slice(&payload[42..74])
        .map_err(|e| WalletError::other(format!("extended key secret invalid: {e}")))?;
    Ok(Xpriv {
        network: bitcoin::NetworkKind::Main,
        depth,
        parent_fingerprint,
        child_number,
        chain_code,
        private_key,
    })
}

fn parse_vericonomy_ext_key(coin: CoinId, secret: &str) -> Result<Xpriv> {
    let trimmed = normalize_hd_master_secret(secret);
    let data = bs58::decode(&trimmed)
        .with_check(None)
        .into_vec()
        .map_err(|e| WalletError::other(format!("invalid extended key encoding: {e}")))?;
    if data.len() != EXT_SECRET_PREFIX_LEN + BIP32_EXTKEY_SIZE {
        return Err(WalletError::other(
            "extended key length is wrong — paste the full master key from Security → Export",
        ));
    }
    let prefix: [u8; EXT_SECRET_PREFIX_LEN] = data[0..EXT_SECRET_PREFIX_LEN]
        .try_into()
        .map_err(|_| WalletError::other("extended key prefix missing"))?;
    if !ext_secret_prefixes(coin).contains(&prefix) {
        return Err(WalletError::other(
            "extended key prefix does not match this chain — export from the same coin (VRM/VRC) in full-node mode",
        ));
    }
    bip32_payload_to_xpriv(&data[EXT_SECRET_PREFIX_LEN..])
}

pub fn parse_root_xpriv(coin: CoinId, secret: &str) -> Result<Xpriv> {
    let trimmed = normalize_hd_master_secret(secret);
    if trimmed.starts_with("xprv") {
        return Xpriv::from_str(&trimmed)
            .map_err(|e| WalletError::other(format!("invalid xprv: {e}")));
    }
    parse_vericonomy_ext_key(coin, &trimmed)
}

#[deprecated(note = "use is_hd_master_secret(coin, secret)")]
pub fn is_xprv_secret(secret: &str) -> bool {
    secret.trim().starts_with("xprv")
}

pub fn derive_address_at(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    index: u32,
) -> Result<String> {
    derive_address_on_chain(coin, seed_secret, bip39_passphrase, HdChain::External, index)
}

pub fn derive_change_address_at(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    index: u32,
) -> Result<String> {
    derive_address_on_chain(coin, seed_secret, bip39_passphrase, HdChain::Internal, index)
}

pub fn derive_address_on_chain(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    chain: HdChain,
    index: u32,
) -> Result<String> {
    let (_, pubkey) = derive_keypair_on_chain(coin, seed_secret, bip39_passphrase, chain, index)?;
    pubkey_to_p2pkh_address(coin, &pubkey)
}

pub fn derive_keypair_at(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    index: u32,
) -> Result<([u8; 32], Vec<u8>)> {
    derive_keypair_on_chain(coin, seed_secret, bip39_passphrase, HdChain::External, index)
}

fn parse_root_xpriv_from_seed(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
) -> Result<Xpriv> {
    if is_hd_master_secret(coin, seed_secret) {
        return parse_root_xpriv(coin, seed_secret);
    }
    let mnemonic = Mnemonic::parse_in(Language::English, seed_secret.trim())
        .map_err(|e| WalletError::other(format!("invalid mnemonic: {e}")))?;
    let seed = mnemonic.to_seed(bip39_passphrase.unwrap_or(""));
    Xpriv::new_master(bitcoin::NetworkKind::Main, &seed)
        .map_err(|e| WalletError::other(format!("master key derivation failed: {e}")))
}

fn derivation_path_for(
    coin: CoinId,
    seed_secret: &str,
    chain: HdChain,
    index: u32,
) -> Result<DerivationPath> {
    if uses_core_hd_paths(coin, seed_secret) {
        let account = ChildNumber::from_hardened_idx(0)
            .map_err(|e| WalletError::other(format!("invalid account index: {e}")))?;
        let chain_idx = ChildNumber::from_hardened_idx(chain.hardened_index())
            .map_err(|e| WalletError::other(format!("invalid chain index: {e}")))?;
        let addr = ChildNumber::from_hardened_idx(index)
            .map_err(|e| WalletError::other(format!("invalid address index: {e}")))?;
        return Ok(DerivationPath::from(vec![account, chain_idx, addr]));
    }
    if chain != HdChain::External {
        return Err(WalletError::other(
            "BIP44 mnemonics only support external receive chain",
        ));
    }
    let coin_type = coin_type_for(coin);
    format!("m/44'/{coin_type}'/0'/0/{index}")
        .parse()
        .map_err(|e| WalletError::other(format!("invalid derivation path: {e}")))
}

pub fn derive_keypair_on_chain(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    chain: HdChain,
    index: u32,
) -> Result<([u8; 32], Vec<u8>)> {
    let secp = Secp256k1::new();
    let xpriv = parse_root_xpriv_from_seed(coin, seed_secret, bip39_passphrase)?;
    let path = derivation_path_for(coin, seed_secret, chain, index)?;
    let child = xpriv
        .derive_priv(&secp, &path)
        .map_err(|e| WalletError::other(format!("derive failed: {e}")))?;
    let secret = child.private_key.secret_bytes();
    let privkey = PrivateKey::new(child.private_key, bitcoin::NetworkKind::Main);
    let pubkey = PublicKey::from_private_key(&secp, &privkey);
    Ok((secret, pubkey.to_bytes()))
}

pub fn derive_wif_at(
    coin: CoinId,
    mnemonic: &str,
    bip39_passphrase: Option<&str>,
    index: u32,
) -> Result<String> {
    let (secret, _) = derive_keypair_at(coin, mnemonic, bip39_passphrase, index)?;
    Ok(secret_bytes_to_wif(secret_key_prefix(coin), &secret))
}

pub fn address_to_script_pubkey(coin: CoinId, address: &str) -> Result<Vec<u8>> {
    let decoded = bs58::decode(address)
        .with_check(Some(pubkey_address_version(coin)))
        .into_vec()
        .map_err(|e| WalletError::InvalidAddress {
            code: WalletError::CODE_INVALID_ADDRESS,
            message: format!("invalid address: {e}"),
        })?;
    if decoded.len() != 21 {
        return Err(WalletError::InvalidAddress {
            code: WalletError::CODE_INVALID_ADDRESS,
            message: "address payload wrong length".into(),
        });
    }
    let pk_hash = &decoded[1..];
    let hash = PubkeyHash::from_slice(pk_hash).map_err(|e| WalletError::InvalidAddress {
        code: WalletError::CODE_INVALID_ADDRESS,
        message: format!("pubkey hash: {e}"),
    })?;
    let script = ScriptBuf::new_p2pkh(&hash);
    Ok(script.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_light_wallet_seed_preserves_mnemonic_words() {
        let phrase = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        assert_eq!(normalize_light_wallet_seed(phrase), phrase);
    }

    #[test]
    fn wif_prefix_matches_chain_profile() {
        let coin = CoinId::Verium;
        let secret = [1u8; 32];
        let _ = secret;
        let wif = derive_wif_at(
            coin,
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            None,
            0,
        )
        .unwrap();
        let decoded = bs58::decode(&wif).with_check(None).into_vec().unwrap();
        assert_eq!(decoded[0], secret_key_prefix(coin));
    }
}
