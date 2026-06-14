//! BIP39 mnemonic generation and BIP32 master key derivation for sethdseed.
//! WIF encoding uses each chain's Base58 secret-key prefix (see chainparams.cpp),
//! supplied by the caller as `secret_prefix` so this crate stays chain-agnostic.

use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::Secp256k1;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Result, WalletCoreError};

#[derive(Debug, Clone, Serialize, Deserialize, ZeroizeOnDrop)]
pub struct RecoveryPhraseBundle {
    pub mnemonic: String,
    pub word_count: u32,
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

/// Encode a compressed secp256k1 secret as chain-correct WIF for `DecodeSecret`
/// / sethdseed. `secret_prefix` is `base58Prefixes[SECRET_KEY]` for the chain.
pub fn secret_bytes_to_wif(secret_prefix: u8, secret: &[u8; 32]) -> String {
    let mut payload = Vec::with_capacity(33);
    payload.extend_from_slice(secret);
    payload.push(1); // compressed
    base58check_encode(secret_prefix, &payload)
}

/// Generate a new 24-word BIP39 mnemonic (256-bit entropy).
pub fn generate_mnemonic() -> Result<RecoveryPhraseBundle> {
    let mut entropy = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut entropy);
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
        .map_err(|e| WalletCoreError::MnemonicGeneration(e.to_string()))?;
    Ok(RecoveryPhraseBundle {
        mnemonic: mnemonic.to_string(),
        word_count: 24,
    })
}

/// Validate a BIP39 mnemonic (checksum included).
pub fn validate_mnemonic(phrase: &str) -> bool {
    Mnemonic::parse_in(Language::English, phrase.trim()).is_ok()
}

/// Derive the BIP32 master extended private key from a mnemonic + optional BIP39 passphrase.
pub fn derive_master_xpriv(phrase: &str, bip39_passphrase: Option<&str>) -> Result<String> {
    let xpriv = master_xpriv(phrase, bip39_passphrase)?;
    Ok(xpriv.to_string())
}

/// Derive master private key WIF in the chain's native format for sethdseed.
pub fn master_xpriv_to_wif(
    secret_prefix: u8,
    phrase: &str,
    bip39_passphrase: Option<&str>,
) -> Result<String> {
    let xpriv = master_xpriv(phrase, bip39_passphrase)?;
    Ok(secret_bytes_to_wif(
        secret_prefix,
        &xpriv.private_key.secret_bytes(),
    ))
}

fn master_xpriv(phrase: &str, bip39_passphrase: Option<&str>) -> Result<Xpriv> {
    let mnemonic = Mnemonic::parse_in(Language::English, phrase.trim())
        .map_err(|e| WalletCoreError::InvalidMnemonic(e.to_string()))?;
    let seed = mnemonic.to_seed(bip39_passphrase.unwrap_or(""));
    Xpriv::new_master(bitcoin::NetworkKind::Main, &seed)
        .map_err(|e| WalletCoreError::MasterKeyDerivation(e.to_string()))
}

/// Pick random word indices (0-based) for verification challenge.
pub fn verification_indices(word_count: u32, count: usize) -> Vec<usize> {
    use rand::seq::SliceRandom;
    let mut indices: Vec<usize> = (0..word_count as usize).collect();
    let mut rng = rand::thread_rng();
    indices.shuffle(&mut rng);
    indices.truncate(count.min(word_count as usize));
    indices.sort_unstable();
    indices
}

/// Verify user-supplied words at given indices.
pub fn verify_words_at_indices(phrase: &str, indices: &[usize], answers: &[String]) -> bool {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if indices.len() != answers.len() {
        return false;
    }
    for (idx, answer) in indices.iter().zip(answers.iter()) {
        if *idx >= words.len() {
            return false;
        }
        if !words[*idx].eq_ignore_ascii_case(answer.trim()) {
            return false;
        }
    }
    true
}

/// Derive a child private key WIF at m/44'/coin_type'/0'/0/index for address preview.
pub fn derive_account_wif(
    secret_prefix: u8,
    phrase: &str,
    bip39_passphrase: Option<&str>,
    coin_type: u32,
    index: u32,
) -> Result<String> {
    let secp = Secp256k1::new();
    let xpriv = master_xpriv(phrase, bip39_passphrase)?;
    let path: DerivationPath = format!("m/44'/{coin_type}'/0'/0/{index}")
        .parse()
        .map_err(|e: bitcoin::bip32::Error| WalletCoreError::DerivationPath(e.to_string()))?;
    let child = xpriv
        .derive_priv(&secp, &path)
        .map_err(|e| WalletCoreError::Derive(e.to_string()))?;
    Ok(secret_bytes_to_wif(
        secret_prefix,
        &child.private_key.secret_bytes(),
    ))
}

/// Zeroize a string in place (best-effort).
pub fn zeroize_string(s: &mut String) {
    unsafe {
        let bytes = s.as_mut_vec();
        bytes.zeroize();
    }
    s.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both Verium and Vericoin mainnet use 128 + 70 = 198.
    const MAINNET_SECRET_PREFIX: u8 = 198;

    #[test]
    fn wif_uses_supplied_prefix_and_compressed_flag() {
        let wif = secret_bytes_to_wif(MAINNET_SECRET_PREFIX, &[1u8; 32]);
        let decoded = bs58::decode(&wif).with_check(None).into_vec().unwrap();
        assert_eq!(decoded[0], MAINNET_SECRET_PREFIX);
        assert_eq!(decoded.len(), 1 + 32 + 1);
        assert_eq!(decoded[33], 1);
    }

    #[test]
    fn master_wif_round_trips_through_decode_secret_layout() {
        let phrase = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        let wif = master_xpriv_to_wif(MAINNET_SECRET_PREFIX, phrase, None).unwrap();
        let decoded = bs58::decode(&wif).with_check(None).into_vec().unwrap();
        assert_eq!(decoded[0], MAINNET_SECRET_PREFIX);
        assert_eq!(decoded.len(), 34);
        assert_eq!(decoded[33], 1);
    }

    #[test]
    fn generate_then_validate_round_trips() {
        let bundle = generate_mnemonic().unwrap();
        assert_eq!(bundle.word_count, 24);
        assert!(validate_mnemonic(&bundle.mnemonic));
        assert!(!validate_mnemonic("not a real mnemonic phrase at all"));
    }

    #[test]
    fn verification_indices_are_unique_sorted_and_in_range() {
        let idx = verification_indices(24, 3);
        assert_eq!(idx.len(), 3);
        assert!(idx.windows(2).all(|w| w[0] < w[1]));
        assert!(idx.iter().all(|i| *i < 24));
    }

    #[test]
    fn verify_words_matches_case_insensitively() {
        let phrase = "alpha bravo charlie delta";
        assert!(verify_words_at_indices(
            phrase,
            &[0, 2],
            &["ALPHA".into(), "Charlie".into()]
        ));
        assert!(!verify_words_at_indices(
            phrase,
            &[0, 2],
            &["alpha".into(), "wrong".into()]
        ));
    }
}
