//! Storage abstractions for wallet secrets, encrypted keystores, and tx caches.

pub mod crypto;
pub mod light_keystore;
pub mod traits;
pub mod types;

pub use crypto::{decrypt_with_passphrase, encrypt_with_passphrase};
pub use light_keystore::{LightKeystoreService, GAP_PRECACHE_MAX, LIGHT_WALLET_RECOVERY_REQUIRED_MSG};
pub use traits::{
    EncryptedBlob, KeystoreStore, SecretStore, TxCache, UtxoCache, WalletIndexStore, WalletStores,
};
pub use types::{
    IndexingProgress, LightKeystore, LightWalletRecord, ScanPhase, SecretBlob, SyncReport,
    WalletStatus, wallet_id,
};
