//! iOS wallet storage paths and store bundle.

use std::path::PathBuf;

use vericonomy_storage::{LightKeystoreService, WalletStores};
use vericonomy_storage_file::{FileKeystoreStore, FileSecretStore};
use vericonomy_storage_sqlite::SqliteWalletCache;

/// Default Application Support directory for Vericonomy Wallet on iOS.
pub fn ios_app_support_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("VericonomyWallet")
}

pub type IosFileSecretStore = FileSecretStore;
pub type IosFileKeystoreStore = FileKeystoreStore<IosFileSecretStore>;
pub type IosSqliteCache = SqliteWalletCache;
pub type IosKeystoreService = LightKeystoreService<IosFileKeystoreStore>;
pub type IosWalletStores = WalletStores<IosFileKeystoreStore, IosSqliteCache, IosSqliteCache>;

pub fn open_ios_wallet_stores(base_dir: Option<PathBuf>) -> IosWalletStores {
    let base = base_dir.unwrap_or_else(ios_app_support_dir);
    let secret = FileSecretStore::new(base.join("secure"));
    let keystore = FileKeystoreStore::new(&base, secret);
    let cache = SqliteWalletCache::new(&base);
    WalletStores::new(keystore, cache.clone(), cache)
}

pub fn open_ios_keystore_service(base_dir: Option<PathBuf>) -> IosKeystoreService {
    let stores = open_ios_wallet_stores(base_dir);
    LightKeystoreService::new(stores.keystore)
}

#[cfg(target_os = "ios")]
pub mod keychain {
    //! iOS Keychain-backed master key (optional upgrade from file secret store).

    use vericonomy_errors::{Result, WalletError};

    const SERVICE: &str = "com.vericonomy.wallet.ios";
    const ACCOUNT: &str = "secret-store-master-v1";

    pub fn save_master_key(data: &[u8]) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| WalletError::Storage {
            code: WalletError::CODE_STORAGE,
            message: e.to_string(),
        })?;
        entry
            .set_password(&hex::encode(data))
            .map_err(|e| WalletError::Storage {
                code: WalletError::CODE_STORAGE,
                message: e.to_string(),
            })
    }

    pub fn load_master_key() -> Result<Option<Vec<u8>>> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| WalletError::Storage {
            code: WalletError::CODE_STORAGE,
            message: e.to_string(),
        })?;
        match entry.get_password() {
            Ok(hex_str) => {
                let bytes = hex::decode(hex_str.trim()).map_err(|e| WalletError::Serde {
                    code: WalletError::CODE_SERDE,
                    message: e.to_string(),
                })?;
                Ok(Some(bytes))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(WalletError::Storage {
                code: WalletError::CODE_STORAGE,
                message: e.to_string(),
            }),
        }
    }
}
