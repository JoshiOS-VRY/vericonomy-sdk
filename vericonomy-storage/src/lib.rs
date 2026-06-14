//! Storage abstractions for wallet secrets, encrypted keystores, and tx caches.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use vericonomy_chain::types::WalletTx;
use vericonomy_errors::Result;

/// Opaque encrypted secret material (mnemonic, xprv, or passphrase-protected blob).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretBlob {
    pub ciphertext: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
}

/// Low-level secret persistence (platform keychain, secure enclave, etc.).
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn load_secret(&self, key: &str) -> Result<Option<SecretBlob>>;
    async fn save_secret(&self, key: &str, blob: &SecretBlob) -> Result<()>;
    async fn delete_secret(&self, key: &str) -> Result<()>;
}

/// Encrypted wallet keystore (BIP39 seed + metadata).
#[async_trait]
pub trait Keystore: Send + Sync {
    async fn is_locked(&self) -> Result<bool>;
    async fn unlock(&self, passphrase: &str) -> Result<()>;
    async fn lock(&self) -> Result<()>;
    async fn mnemonic(&self) -> Result<String>;
    async fn set_mnemonic(&self, mnemonic: &str, passphrase: &str) -> Result<()>;
}

/// Cached transaction history for offline display.
#[async_trait]
pub trait TxCache: Send + Sync {
    async fn load_history(&self, wallet_id: &str) -> Result<Vec<WalletTx>>;
    async fn save_history(&self, wallet_id: &str, txs: &[WalletTx]) -> Result<()>;
    async fn clear_history(&self, wallet_id: &str) -> Result<()>;
}
