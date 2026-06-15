//! Encrypted JSON keystore on disk (compatible with Tauri light-keystore layout).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json;
use vericonomy_errors::{Result, WalletError};
use vericonomy_storage::{KeystoreStore, LightKeystore, SecretBlob, SecretStore};

const KEYSTORE_LABEL: &str = "light-wallet-keystore";

pub struct FileKeystoreStore<S: SecretStore> {
    base_dir: PathBuf,
    secret_store: S,
}

impl<S: SecretStore> FileKeystoreStore<S> {
    pub fn new(base_dir: impl AsRef<Path>, secret_store: S) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            secret_store,
        }
    }

    fn secure_dir(&self) -> PathBuf {
        self.base_dir.join("secure")
    }

    fn encrypted_path(&self) -> PathBuf {
        self.secure_dir().join(format!("{KEYSTORE_LABEL}.enc"))
    }

    fn plaintext_path(&self) -> PathBuf {
        self.base_dir.join("light-keystore.json")
    }
}

#[async_trait]
impl<S: SecretStore + Send + Sync> KeystoreStore for FileKeystoreStore<S> {
    async fn load_keystore(&self) -> Result<Option<LightKeystore>> {
        if let Some(blob) = self.secret_store.load_secret(KEYSTORE_LABEL).await? {
            let json = String::from_utf8(blob.ciphertext).map_err(|e| WalletError::Serde {
                code: WalletError::CODE_SERDE,
                message: e.to_string(),
            })?;
            let store: LightKeystore = serde_json::from_str(&json).map_err(|e| WalletError::Serde {
                code: WalletError::CODE_SERDE,
                message: e.to_string(),
            })?;
            return Ok(Some(store));
        }
        let path = self.plaintext_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| WalletError::Io {
            code: WalletError::CODE_IO,
            message: e.to_string(),
        })?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let store: LightKeystore = serde_json::from_str(&raw).map_err(|e| WalletError::Serde {
            code: WalletError::CODE_SERDE,
            message: e.to_string(),
        })?;
        Ok(Some(store))
    }

    async fn save_keystore(&self, store: &LightKeystore) -> Result<()> {
        let json = serde_json::to_vec(store).map_err(|e| WalletError::Serde {
            code: WalletError::CODE_SERDE,
            message: e.to_string(),
        })?;
        if let Some(parent) = self.secure_dir().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::create_dir_all(self.secure_dir());
        self.secret_store
            .save_secret(
                KEYSTORE_LABEL,
                &SecretBlob {
                    ciphertext: json,
                    salt: Vec::new(),
                    nonce: Vec::new(),
                },
            )
            .await?;
        Ok(())
    }

    async fn keystore_exists(&self) -> Result<bool> {
        Ok(self.encrypted_path().exists() || self.plaintext_path().exists())
    }
}

/// Simple file-backed secret store (master key file for development / iOS file vault).
pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

#[async_trait]
impl SecretStore for FileSecretStore {
    async fn load_secret(&self, key: &str) -> Result<Option<SecretBlob>> {
        let path = self.path.join(format!("{key}.enc"));
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path).map_err(|e| WalletError::Io {
            code: WalletError::CODE_IO,
            message: e.to_string(),
        })?;
        Ok(Some(SecretBlob {
            ciphertext: data,
            salt: Vec::new(),
            nonce: Vec::new(),
        }))
    }

    async fn save_secret(&self, key: &str, blob: &SecretBlob) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WalletError::Io {
                code: WalletError::CODE_IO,
                message: e.to_string(),
            })?;
        }
        let path = self.path.join(format!("{key}.enc"));
        std::fs::write(&path, &blob.ciphertext).map_err(|e| WalletError::Io {
            code: WalletError::CODE_IO,
            message: e.to_string(),
        })?;
        Ok(())
    }

    async fn delete_secret(&self, key: &str) -> Result<()> {
        let path = self.path.join(format!("{key}.enc"));
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| WalletError::Io {
                code: WalletError::CODE_IO,
                message: e.to_string(),
            })?;
        }
        Ok(())
    }
}
