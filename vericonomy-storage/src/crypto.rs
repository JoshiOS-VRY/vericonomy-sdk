//! Passphrase sealing (Argon2id + AES-256-GCM) for light-wallet mnemonics.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use vericonomy_errors::{Result, WalletError};

const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;

fn derive_key_from_passphrase(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(19 * 1024, 2, 1, Some(32))
        .map_err(|e| WalletError::other(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| WalletError::other(format!("argon2 derive: {e}")))?;
    Ok(out)
}

pub fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &str) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let key = derive_key_from_passphrase(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| WalletError::other(format!("cipher init: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| WalletError::other(format!("encrypt failed: {e}")))?;
    Ok((ciphertext, salt.to_vec(), nonce_bytes.to_vec()))
}

pub fn decrypt_with_passphrase(
    ciphertext: &[u8],
    salt: &[u8],
    nonce_bytes: &[u8],
    passphrase: &str,
) -> Result<Vec<u8>> {
    let key = derive_key_from_passphrase(passphrase, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| WalletError::other(format!("cipher init: {e}")))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| WalletError::WrongPassphrase {
            code: WalletError::CODE_WRONG_PASSPHRASE,
        })
}
