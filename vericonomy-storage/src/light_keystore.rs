//! Light-wallet keystore operations (mnemonic sealing, unlock sessions, indexing metadata).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use hex;
use zeroize::Zeroizing;

use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd::{address_to_script_pubkey, derive_address_on_chain, uses_core_hd_paths, HdChain};
use vericonomy_wallet_core::validate_mnemonic;

use crate::crypto::{decrypt_with_passphrase, encrypt_with_passphrase};
use crate::traits::{KeystoreStore, WalletIndexStore};
use crate::types::{IndexingProgress, LightKeystore, LightWalletRecord};

pub const GAP_PRECACHE_MAX: u32 = 80;
pub const LIGHT_WALLET_RECOVERY_REQUIRED_MSG: &str =
    "Light wallet metadata is on this device but wallet keys are missing from the local store. \
     Import your recovery phrase from Setup to restore access.";

static SESSION_MNEMONICS: Mutex<Option<HashMap<String, Zeroizing<String>>>> = Mutex::new(None);

fn session_map() -> std::sync::MutexGuard<'static, Option<HashMap<String, Zeroizing<String>>>> {
    let mut guard = SESSION_MNEMONICS.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

pub struct LightKeystoreService<K: KeystoreStore> {
    store: K,
}

impl<K: KeystoreStore> LightKeystoreService<K> {
    pub fn new(store: K) -> Self {
        Self { store }
    }

    pub async fn load(&self) -> Result<LightKeystore> {
        Ok(self
            .store
            .load_keystore()
            .await?
            .unwrap_or_default())
    }

    pub async fn save(&self, store: &LightKeystore) -> Result<()> {
        self.store.save_keystore(store).await
    }

    pub async fn wallet_exists(&self, coin: CoinId) -> Result<bool> {
        let store = self.load().await?;
        Ok(store.wallets.contains_key(coin.as_str()))
    }

    pub async fn create_wallet(
        &self,
        coin: CoinId,
        mnemonic: &str,
        passphrase: &str,
        label: Option<&str>,
    ) -> Result<()> {
        if !validate_mnemonic(mnemonic) {
            return Err(WalletError::InvalidMnemonic {
                code: WalletError::CODE_INVALID_MNEMONIC,
                message: "invalid mnemonic phrase".into(),
            });
        }
        let mut store = self.load().await?;
        if store.wallets.contains_key(coin.as_str()) {
            return Err(WalletError::other(format!(
                "{} light wallet already exists",
                coin.as_str()
            )));
        }
        let record = seal_wallet_record(coin, mnemonic, passphrase, label, None)?;
        store.wallets.insert(coin.as_str().to_string(), record);
        persist_unlock_in_store(&mut store, coin, mnemonic, 24 * 60 * 60)?;
        self.save(&store).await?;
        precache_light_wallet_scripts(coin, mnemonic, &mut store).await?;
        self.save(&store).await?;
        Ok(())
    }

    pub async fn import_wallet(
        &self,
        coin: CoinId,
        seed_secret: &str,
        passphrase: &str,
        label: Option<&str>,
    ) -> Result<()> {
        clear_unlock_session(coin);
        let mut store = self.load().await?;
        store.unlocked_until_by_coin.remove(coin.as_str());
        let created_at = store.wallets.get(coin.as_str()).map(|r| r.created_at);
        let record = seal_wallet_record(coin, seed_secret, passphrase, label, created_at)?;
        store.wallets.insert(coin.as_str().to_string(), record);
        persist_unlock_in_store(&mut store, coin, seed_secret, 24 * 60 * 60)?;
        precache_light_wallet_scripts(coin, seed_secret, &mut store).await?;
        self.save(&store).await?;
        Ok(())
    }

    pub async fn unlock_wallet(&self, coin: CoinId, passphrase: &str, seconds: u32) -> Result<()> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        let phrase = decrypt_mnemonic(record, passphrase)?;
        persist_unlock_in_store(&mut store, coin, &phrase, seconds)?;
        self.save(&store).await
    }

    pub async fn lock_wallet(&self, coin: CoinId) -> Result<()> {
        clear_unlock_session(coin);
        let mut store = self.load().await?;
        if store.unlocked_until_by_coin.remove(coin.as_str()).is_some() {
            self.save(&store).await?;
        }
        Ok(())
    }

    pub fn signing_session_active(&self, coin: CoinId) -> bool {
        session_map()
            .as_ref()
            .and_then(|m| m.get(coin.as_str()).map(|_| true))
            .unwrap_or(false)
    }

    pub async fn is_unlocked(&self, coin: CoinId) -> Result<bool> {
        let store = self.load().await?;
        let now = unix_now();
        Ok(store
            .unlocked_until_by_coin
            .get(coin.as_str())
            .map(|u| *u > now)
            .unwrap_or(false))
    }

    pub async fn unlocked_mnemonic(&self, coin: CoinId, passphrase: &str) -> Result<String> {
        if let Some(m) = session_map().as_ref() {
            if let Some(phrase) = m.get(coin.as_str()) {
                return Ok(phrase.to_string());
            }
        }
        if passphrase.is_empty() {
            return Err(WalletError::LockedWallet {
                code: WalletError::CODE_LOCKED_WALLET,
            });
        }
        let store = self.load().await?;
        let record = store
            .wallets
            .get(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        let phrase = decrypt_mnemonic(record, passphrase)?;
        if self.is_unlocked(coin).await? {
            if let Some(m) = session_map().as_mut() {
                m.insert(coin.as_str().to_string(), Zeroizing::new(phrase.clone()));
            }
        }
        Ok(phrase)
    }

    pub async fn bump_receive_index(&self, coin: CoinId) -> Result<u32> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        let idx = record.next_receive_index;
        record.next_receive_index = idx.saturating_add(1);
        self.save(&store).await?;
        Ok(idx)
    }

    pub async fn peek_receive_index(&self, coin: CoinId) -> Result<u32> {
        let store = self.load().await?;
        store
            .wallets
            .get(coin.as_str())
            .map(|r| r.next_receive_index)
            .ok_or_else(|| WalletError::other("light wallet not found"))
    }

    /// Advance the HD receive counter when gap scan discovers higher-used indices (e.g. after import).
    pub async fn set_receive_index_at_least(&self, coin: CoinId, index: u32) -> Result<()> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        if index > record.next_receive_index {
            record.next_receive_index = index;
            self.save(&store).await?;
        }
        Ok(())
    }

    pub async fn funded_script_hexes(&self, coin: CoinId) -> Result<Vec<String>> {
        let store = self.load().await?;
        Ok(store
            .wallets
            .get(coin.as_str())
            .map(|r| r.funded_script_hexes.clone())
            .unwrap_or_default())
    }

    pub async fn set_funded_script_hexes(&self, coin: CoinId, scripts: &[String]) -> Result<()> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        record.funded_script_hexes = scripts.to_vec();
        self.save(&store).await
    }

    pub async fn register_funded_script_hex(&self, coin: CoinId, script_hex: &str) -> Result<()> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        if !record
            .funded_script_hexes
            .iter()
            .any(|s| s.eq_ignore_ascii_case(script_hex))
        {
            record.funded_script_hexes.push(script_hex.to_string());
            self.save(&store).await?;
        }
        Ok(())
    }

    pub async fn cached_script_hexes(&self, coin: CoinId) -> Result<Vec<String>> {
        let store = self.load().await?;
        Ok(store
            .wallets
            .get(coin.as_str())
            .map(|r| r.cached_script_hexes.clone())
            .unwrap_or_default())
    }

    pub async fn set_cached_script_hexes(
        &self,
        coin: CoinId,
        scripts: &[String],
        scan_complete: bool,
    ) -> Result<()> {
        if scripts.is_empty() {
            return Ok(());
        }
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        record.cached_script_hexes = scripts.to_vec();
        if scan_complete {
            record.addresses_scan_complete = true;
        }
        self.save(&store).await
    }

    pub async fn indexing_progress(&self, coin: CoinId) -> Result<IndexingProgress> {
        let store = self.load().await?;
        Ok(store
            .wallets
            .get(coin.as_str())
            .map(|r| {
                let mut progress = IndexingProgress {
                    precache_offset: r.index_precache_offset,
                    gap_external: r.index_gap_external,
                    gap_external_done: r.index_gap_external_done,
                    gap_internal: r.index_gap_internal,
                    gap_internal_done: r.index_gap_internal_done,
                };
                if !progress.gap_external_done && progress.gap_internal > 0 {
                    progress.gap_external_done = true;
                }
                progress
            })
            .unwrap_or_default())
    }

    pub async fn set_indexing_progress(
        &self,
        coin: CoinId,
        progress: IndexingProgress,
    ) -> Result<()> {
        let mut store = self.load().await?;
        let record = store
            .wallets
            .get_mut(coin.as_str())
            .ok_or_else(|| WalletError::other("light wallet not found"))?;
        if record.index_precache_offset == progress.precache_offset
            && record.index_gap_external == progress.gap_external
            && record.index_gap_external_done == progress.gap_external_done
            && record.index_gap_internal == progress.gap_internal
            && record.index_gap_internal_done == progress.gap_internal_done
        {
            return Ok(());
        }
        record.index_precache_offset = progress.precache_offset;
        record.index_gap_external = progress.gap_external;
        record.index_gap_external_done = progress.gap_external_done;
        record.index_gap_internal = progress.gap_internal;
        record.index_gap_internal_done = progress.gap_internal_done;
        self.save(&store).await
    }

    pub async fn needs_full_address_scan(&self, coin: CoinId) -> Result<bool> {
        let store = self.load().await?;
        Ok(store
            .wallets
            .get(coin.as_str())
            .map(|r| !r.addresses_scan_complete)
            .unwrap_or(false))
    }

    pub async fn mark_address_scan_complete(&self, coin: CoinId) -> Result<()> {
        let mut store = self.load().await?;
        let Some(record) = store.wallets.get_mut(coin.as_str()) else {
            return Ok(());
        };
        record.addresses_scan_complete = true;
        record.index_precache_offset = 0;
        record.index_gap_external = 0;
        record.index_gap_external_done = true;
        record.index_gap_internal = 0;
        record.index_gap_internal_done = true;
        self.save(&store).await
    }

    pub async fn mark_address_scan_incomplete(&self, coin: CoinId) -> Result<()> {
        let mut store = self.load().await?;
        let Some(record) = store.wallets.get_mut(coin.as_str()) else {
            return Ok(());
        };
        record.addresses_scan_complete = false;
        record.index_precache_offset = 0;
        record.index_gap_external = 0;
        record.index_gap_external_done = false;
        record.index_gap_internal = 0;
        record.index_gap_internal_done = false;
        self.save(&store).await
    }

    pub async fn reset_indexing_progress(&self, coin: CoinId) -> Result<()> {
        self.set_indexing_progress(
            coin,
            IndexingProgress {
                precache_offset: 0,
                gap_external: 0,
                gap_external_done: false,
                gap_internal: 0,
                gap_internal_done: false,
            },
        )
        .await
    }
}

#[async_trait]
impl<K: KeystoreStore> WalletIndexStore for LightKeystoreService<K> {
    async fn funded_script_hexes(&self, coin: CoinId) -> Result<Vec<String>> {
        LightKeystoreService::funded_script_hexes(self, coin).await
    }

    async fn set_funded_script_hexes(&self, coin: CoinId, scripts: &[String]) -> Result<()> {
        LightKeystoreService::set_funded_script_hexes(self, coin, scripts).await
    }

    async fn register_funded_script_hex(&self, coin: CoinId, script_hex: &str) -> Result<()> {
        LightKeystoreService::register_funded_script_hex(self, coin, script_hex).await
    }

    async fn cached_script_hexes(&self, coin: CoinId) -> Result<Vec<String>> {
        LightKeystoreService::cached_script_hexes(self, coin).await
    }

    async fn set_cached_script_hexes(
        &self,
        coin: CoinId,
        scripts: &[String],
        scan_complete: bool,
    ) -> Result<()> {
        LightKeystoreService::set_cached_script_hexes(self, coin, scripts, scan_complete).await
    }

    async fn indexing_progress(&self, coin: CoinId) -> Result<IndexingProgress> {
        LightKeystoreService::indexing_progress(self, coin).await
    }

    async fn set_indexing_progress(&self, coin: CoinId, progress: IndexingProgress) -> Result<()> {
        LightKeystoreService::set_indexing_progress(self, coin, progress).await
    }

    async fn scan_complete(&self, coin: CoinId) -> Result<bool> {
        Ok(!self.needs_full_address_scan(coin).await?)
    }

    async fn mark_scan_complete(&self, coin: CoinId) -> Result<()> {
        self.mark_address_scan_complete(coin).await
    }

    async fn mark_scan_incomplete(&self, coin: CoinId) -> Result<()> {
        self.mark_address_scan_incomplete(coin).await
    }

    async fn next_receive_index(&self, coin: CoinId) -> Result<u32> {
        self.peek_receive_index(coin).await
    }

    async fn bump_receive_index(&self, coin: CoinId) -> Result<u32> {
        LightKeystoreService::bump_receive_index(self, coin).await
    }

    async fn set_receive_index_at_least(&self, coin: CoinId, index: u32) -> Result<()> {
        LightKeystoreService::set_receive_index_at_least(self, coin, index).await
    }

    async fn wallet_exists(&self, coin: CoinId) -> Result<bool> {
        LightKeystoreService::wallet_exists(self, coin).await
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn seal_wallet_record(
    coin: CoinId,
    seed_secret: &str,
    passphrase: &str,
    label: Option<&str>,
    preserve_created_at: Option<u64>,
) -> Result<LightWalletRecord> {
    let (encrypted, salt, nonce) = encrypt_with_passphrase(seed_secret.as_bytes(), passphrase)?;
    Ok(LightWalletRecord {
        coin: coin.as_str().to_string(),
        encrypted_mnemonic: hex::encode(encrypted),
        salt: hex::encode(salt),
        nonce: hex::encode(nonce),
        created_at: preserve_created_at.unwrap_or_else(unix_now),
        next_receive_index: 0,
        label: label.unwrap_or("").to_string(),
        cached_script_hexes: Vec::new(),
        addresses_scan_complete: false,
        funded_script_hexes: Vec::new(),
        index_precache_offset: 0,
        index_gap_external: 0,
        index_gap_external_done: false,
        index_gap_internal: 0,
        index_gap_internal_done: false,
    })
}

fn decrypt_mnemonic(record: &LightWalletRecord, passphrase: &str) -> Result<String> {
    let encrypted = hex::decode(record.encrypted_mnemonic.trim())
        .map_err(|e| WalletError::other(format!("keystore corrupt: {e}")))?;
    let salt = hex::decode(record.salt.trim())
        .map_err(|e| WalletError::other(format!("keystore corrupt: {e}")))?;
    let nonce = hex::decode(record.nonce.trim())
        .map_err(|e| WalletError::other(format!("keystore corrupt: {e}")))?;
    let plain = decrypt_with_passphrase(&encrypted, &salt, &nonce, passphrase)?;
    String::from_utf8(plain).map_err(|e| WalletError::other(format!("mnemonic utf8: {e}")))
}

fn persist_unlock_in_store(
    store: &mut LightKeystore,
    coin: CoinId,
    seed_secret: &str,
    seconds: u32,
) -> Result<()> {
    let until = unix_now() + seconds as u64;
    store
        .unlocked_until_by_coin
        .insert(coin.as_str().to_string(), until);
    if let Some(m) = session_map().as_mut() {
        m.insert(
            coin.as_str().to_string(),
            Zeroizing::new(seed_secret.to_string()),
        );
    }
    Ok(())
}

fn clear_unlock_session(coin: CoinId) {
    if let Some(m) = session_map().as_mut() {
        m.remove(coin.as_str());
    }
}

async fn precache_light_wallet_scripts(
    coin: CoinId,
    seed_secret: &str,
    store: &mut LightKeystore,
) -> Result<()> {
    let mut scripts = Vec::new();
    let chains: &[HdChain] = if uses_core_hd_paths(coin, seed_secret) {
        &[HdChain::External, HdChain::Internal]
    } else {
        &[HdChain::External]
    };
    for &chain in chains {
        for index in 0..=GAP_PRECACHE_MAX {
            let addr = derive_address_on_chain(coin, seed_secret, None, chain, index)?;
            let script = address_to_script_pubkey(coin, &addr)?;
            scripts.push(hex::encode(script));
        }
    }
    if let Some(record) = store.wallets.get_mut(coin.as_str()) {
        record.cached_script_hexes = scripts;
    }
    Ok(())
}
