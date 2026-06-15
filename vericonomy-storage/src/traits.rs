//! Storage trait surface for platform implementations.

use async_trait::async_trait;

use vericonomy_chain::types::{Utxo, WalletTx};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;

use crate::types::{IndexingProgress, LightKeystore};

pub use crate::types::SecretBlob;

/// Opaque encrypted secret material (mnemonic envelope blob).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EncryptedBlob {
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

/// Encrypted wallet keystore persistence.
#[async_trait]
pub trait KeystoreStore: Send + Sync {
    async fn load_keystore(&self) -> Result<Option<LightKeystore>>;
    async fn save_keystore(&self, store: &LightKeystore) -> Result<()>;
    async fn keystore_exists(&self) -> Result<bool>;
}

/// SQLite-backed UTXO cache + sync metadata.
#[async_trait]
pub trait UtxoCache: Send + Sync {
    async fn list_utxos(&self, coin: CoinId) -> Result<Vec<Utxo>>;
    async fn replace_utxos(&self, coin: CoinId, utxos: &[Utxo]) -> Result<bool>;
    async fn upsert_utxo(&self, coin: CoinId, utxo: &Utxo) -> Result<()>;
    async fn remove_utxo(&self, coin: CoinId, txid: &str, vout: u32) -> Result<bool>;
    async fn get_meta(&self, coin: CoinId, key: &str) -> Result<Option<String>>;
    async fn set_meta(&self, coin: CoinId, key: &str, value: &str) -> Result<()>;
    async fn clear_coin(&self, coin: CoinId) -> Result<()>;
}

/// Cached transaction history for offline display.
#[async_trait]
pub trait TxCache: Send + Sync {
    async fn load_history(&self, coin: CoinId, limit: usize) -> Result<Vec<WalletTx>>;
    async fn replace_history(&self, coin: CoinId, txs: &[WalletTx]) -> Result<()>;
    async fn clear_history(&self, coin: CoinId) -> Result<()>;
}

/// Per-wallet HD indexing metadata (stored inside keystore record, exposed for facade).
#[async_trait]
pub trait WalletIndexStore: Send + Sync {
    async fn funded_script_hexes(&self, coin: CoinId) -> Result<Vec<String>>;
    async fn set_funded_script_hexes(&self, coin: CoinId, scripts: &[String]) -> Result<()>;
    async fn register_funded_script_hex(&self, coin: CoinId, script_hex: &str) -> Result<()>;
    async fn cached_script_hexes(&self, coin: CoinId) -> Result<Vec<String>>;
    async fn set_cached_script_hexes(&self, coin: CoinId, scripts: &[String], scan_complete: bool)
        -> Result<()>;
    async fn indexing_progress(&self, coin: CoinId) -> Result<IndexingProgress>;
    async fn set_indexing_progress(&self, coin: CoinId, progress: IndexingProgress) -> Result<()>;
    async fn scan_complete(&self, coin: CoinId) -> Result<bool>;
    async fn mark_scan_complete(&self, coin: CoinId) -> Result<()>;
    async fn mark_scan_incomplete(&self, coin: CoinId) -> Result<()>;
    async fn next_receive_index(&self, coin: CoinId) -> Result<u32>;
    async fn bump_receive_index(&self, coin: CoinId) -> Result<u32>;
    async fn set_receive_index_at_least(&self, coin: CoinId, index: u32) -> Result<()>;
    async fn wallet_exists(&self, coin: CoinId) -> Result<bool>;
}

/// Bundle of platform stores for a light wallet session.
pub struct WalletStores<K, U, T>
where
    K: KeystoreStore,
    U: UtxoCache,
    T: TxCache,
{
    pub keystore: K,
    pub utxo_cache: U,
    pub tx_cache: T,
}

impl<K, U, T> WalletStores<K, U, T>
where
    K: KeystoreStore,
    U: UtxoCache,
    T: TxCache,
{
    pub fn new(keystore: K, utxo_cache: U, tx_cache: T) -> Self {
        Self {
            keystore,
            utxo_cache,
            tx_cache,
        }
    }
}
