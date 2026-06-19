//! Primary light-wallet session API for mobile shells.

use std::sync::Arc;

use tokio::sync::RwLock;
use vericonomy_chain::electrum::ElectrumLightClient;
use vericonomy_chain::types::{Utxo, WalletBalance, WalletTx};
use vericonomy_chain::ChainBackend;
use vericonomy_chain_params::{CoinId, NetworkMode};
use vericonomy_errors::Result;
use vericonomy_hd::{derive_address_at, derive_change_address_at, uses_core_hd_paths};
use vericonomy_storage::{
    LightKeystoreService, SyncReport, TxCache, UtxoCache, WalletIndexStore, WalletStatus,
};
use vericonomy_wallet_engine::DEFAULT_TX_FEE_COINS_PER_KB;

use crate::history::HistorySource;
use crate::send::{send_payment, SendPaymentParams, SendResult};
use crate::sync_engine::LightSyncEngine;
use crate::wallet_cache::{apply_local_send_cache_update, balance_from_utxo_cache};

pub struct LightWalletSession<K, U, T>
where
    K: vericonomy_storage::KeystoreStore + Send + Sync + 'static,
    U: UtxoCache + Send + Sync + 'static,
    T: TxCache + Send + Sync + 'static,
{
    coin: CoinId,
    backend: RwLock<ElectrumLightClient>,
    electrum_servers: RwLock<Vec<String>>,
    keystore: Arc<LightKeystoreService<K>>,
    sync: LightSyncEngine<K, U, T>,
    utxo_cache: Arc<U>,
    tx_cache: Arc<T>,
}

impl<K, U, T> LightWalletSession<K, U, T>
where
    K: vericonomy_storage::KeystoreStore + Send + Sync + 'static,
    U: UtxoCache + Send + Sync + 'static,
    T: TxCache + Send + Sync + 'static,
{
    pub fn new(
        coin: CoinId,
        electrum_servers: &[String],
        keystore: LightKeystoreService<K>,
        utxo_cache: U,
        tx_cache: T,
    ) -> Result<Self> {
        let backend = ElectrumLightClient::new(coin, electrum_servers)?;
        let keystore = Arc::new(keystore);
        let utxo_cache = Arc::new(utxo_cache);
        let tx_cache = Arc::new(tx_cache);
        let sync = LightSyncEngine::new(
            Arc::clone(&keystore),
            Arc::clone(&utxo_cache),
            Arc::clone(&tx_cache),
        );
        Ok(Self {
            coin,
            backend: RwLock::new(backend),
            electrum_servers: RwLock::new(electrum_servers.to_vec()),
            keystore,
            sync,
            utxo_cache,
            tx_cache,
        })
    }

    pub fn with_default_servers(
        coin: CoinId,
        keystore: LightKeystoreService<K>,
        utxo_cache: U,
        tx_cache: T,
    ) -> Result<Self> {
        let servers = coin
            .profile()
            .default_electrum_servers(NetworkMode::Mainnet);
        Self::new(coin, &servers, keystore, utxo_cache, tx_cache)
    }

    pub fn coin(&self) -> CoinId {
        self.coin
    }

    pub async fn create_wallet(
        &self,
        mnemonic: &str,
        passphrase: &str,
        label: Option<&str>,
    ) -> Result<()> {
        self.keystore
            .create_wallet(self.coin, mnemonic, passphrase, label)
            .await
    }

    pub async fn import_wallet(
        &self,
        seed_secret: &str,
        passphrase: &str,
        label: Option<&str>,
    ) -> Result<()> {
        self.keystore
            .import_wallet(self.coin, seed_secret, passphrase, label)
            .await?;
        self.utxo_cache.clear_coin(self.coin).await?;
        Ok(())
    }

    pub async fn unlock(&self, passphrase: &str, seconds: u32) -> Result<()> {
        self.keystore
            .unlock_wallet(self.coin, passphrase, seconds)
            .await
    }

    pub async fn lock(&self) -> Result<()> {
        self.keystore.lock_wallet(self.coin).await
    }

    pub async fn exists(&self) -> Result<bool> {
        self.keystore.wallet_exists(self.coin).await
    }

    pub async fn status(&self) -> Result<WalletStatus> {
        let backend = self.backend.read().await;
        self.sync.wallet_status(self.coin, &*backend).await
    }

    pub async fn sync<H: HistorySource>(&self, history: &H) -> Result<SyncReport> {
        let backend = self.backend.read().await;
        self.sync.sync(self.coin, &*backend, history).await
    }

    pub async fn refresh_balance(&self) -> Result<()> {
        let backend = self.backend.read().await;
        self.sync
            .refresh_balance(self.coin, &*backend)
            .await
    }

    pub async fn refresh_pending<H: HistorySource>(&self, history: &H) -> Result<()> {
        let backend = self.backend.read().await;
        self.sync
            .refresh_pending(self.coin, &*backend, history)
            .await
    }

    pub async fn rescan(&self) -> Result<()> {
        self.sync
            .begin_manual_rescan(self.coin, self.utxo_cache.as_ref())
            .await
    }

    pub async fn balance(&self) -> Result<WalletBalance> {
        balance_from_utxo_cache(self.utxo_cache.as_ref(), self.coin).await
    }

    pub async fn new_receive_address(&self, passphrase: &str) -> Result<String> {
        if !self.keystore.is_unlocked(self.coin).await? {
            return Err(vericonomy_errors::WalletError::LockedWallet {
                code: vericonomy_errors::WalletError::CODE_LOCKED_WALLET,
            });
        }
        let phrase = self.keystore.unlocked_mnemonic(self.coin, passphrase).await?;
        let index = self.keystore.bump_receive_index(self.coin).await?;
        derive_address_at(self.coin, &phrase, None, index)
    }

    /// External receive addresses derived for this wallet (on-chain + generated on this device).
    pub async fn list_receive_addresses(&self) -> Result<Vec<String>> {
        use std::collections::BTreeSet;

        if !self.keystore.is_unlocked(self.coin).await? {
            return Ok(Vec::new());
        }
        let phrase = match self.keystore.unlocked_mnemonic(self.coin, "").await {
            Ok(phrase) => phrase,
            Err(_) => return Ok(Vec::new()),
        };

        let mut addresses = BTreeSet::new();
        let next_idx = self.keystore.next_receive_index(self.coin).await?;
        for idx in 0..next_idx {
            addresses.insert(derive_address_at(self.coin, &phrase, None, idx)?);
        }

        let funded = self.keystore.funded_script_hexes(self.coin).await?;
        for script in &funded {
            if let Some((_, addr)) = crate::gap_scan::external_receive_index_for_script(
                self.coin,
                &phrase,
                None,
                script,
            )? {
                addresses.insert(addr);
            }
        }

        Ok(addresses.into_iter().collect())
    }

    pub async fn list_spendable_utxos(&self, passphrase: &str) -> Result<Vec<Utxo>> {
        let backend = self.backend.read().await;
        self.sync
            .list_spendable_utxos(self.coin, &*backend, passphrase)
            .await
    }

    pub async fn list_transactions<H: HistorySource>(
        &self,
        history: &H,
        limit: usize,
    ) -> Result<Vec<WalletTx>> {
        let backend = self.backend.read().await;
        self.sync
            .list_transactions(self.coin, &*backend, history, limit)
            .await
    }

    pub async fn send_to_address<H: HistorySource>(
        &self,
        history: &H,
        recipient: &str,
        amount_sats: i64,
        fee_rate_coins_per_kb: Option<f64>,
        passphrase: &str,
    ) -> Result<SendResult> {
        let mut last_err: Option<vericonomy_errors::WalletError> = None;
        for attempt in 0..2 {
            match self
                .send_to_address_once(
                    history,
                    recipient,
                    amount_sats,
                    fee_rate_coins_per_kb,
                    passphrase,
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(e) if attempt == 0 && e.is_electrum_tx_lookup_failure() => {
                    last_err = Some(e);
                    self.rotate_electrum_server().await?;
                    let backend = self.backend.read().await;
                    let _ = self
                        .sync
                        .list_spendable_utxos(self.coin, &*backend, passphrase)
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            vericonomy_errors::WalletError::other("send failed after electrum retry")
        }))
    }

    async fn send_to_address_once<H: HistorySource>(
        &self,
        _history: &H,
        recipient: &str,
        amount_sats: i64,
        fee_rate_coins_per_kb: Option<f64>,
        passphrase: &str,
    ) -> Result<SendResult> {
        let phrase = self.keystore.unlocked_mnemonic(self.coin, passphrase).await?;
        let backend = self.backend.read().await;
        let utxos = self
            .sync
            .list_spendable_utxos(self.coin, &*backend, passphrase)
            .await?;
        let change_idx = self.keystore.peek_receive_index(self.coin).await?.saturating_sub(1);
        let change_address = if uses_core_hd_paths(self.coin, &phrase) {
            derive_change_address_at(self.coin, &phrase, None, change_idx)?
        } else {
            derive_address_at(self.coin, &phrase, None, change_idx)?
        };
        let result = send_payment(
            &*backend,
            self.coin,
            &phrase,
            SendPaymentParams {
                recipient: recipient.to_string(),
                amount_sats,
                fee_rate_coins_per_kb: fee_rate_coins_per_kb
                    .unwrap_or(DEFAULT_TX_FEE_COINS_PER_KB),
                utxos,
                change_address: change_address.clone(),
                bip39_passphrase: None,
            },
        )
        .await?;
        apply_local_send_cache_update(
            self.utxo_cache.as_ref(),
            self.tx_cache.as_ref(),
            self.keystore.as_ref(),
            self.coin,
            &result.spent_utxos,
            &result.txid,
            &result.raw_hex,
            result.change_sats,
            &change_address,
        )
        .await?;
        Ok(result)
    }

    pub async fn estimate_fee(&self, target_blocks: u32) -> Result<f64> {
        let backend = self.backend.read().await;
        let rate = backend.estimate_fee(target_blocks).await?;
        Ok(rate.coins_per_kb)
    }

    pub async fn set_electrum_servers(&self, servers: Vec<String>) -> Result<()> {
        if servers.is_empty() {
            return Err(vericonomy_errors::WalletError::other(
                "electrum server list cannot be empty",
            ));
        }
        *self.electrum_servers.write().await = servers.clone();
        *self.backend.write().await = ElectrumLightClient::new(self.coin, &servers)?;
        Ok(())
    }

    async fn rotate_electrum_server(&self) -> Result<()> {
        let mut servers = self.configured_electrum_servers().await;
        if servers.len() <= 1 {
            return Ok(());
        }
        let first = servers.remove(0);
        servers.push(first);
        self.set_electrum_servers(servers).await
    }

    pub fn default_electrum_servers(&self) -> Vec<String> {
        self.coin
            .profile()
            .default_electrum_servers(NetworkMode::Mainnet)
    }

    pub async fn configured_electrum_servers(&self) -> Vec<String> {
        self.electrum_servers.read().await.clone()
    }

    pub async fn server_status(&self) -> Option<vericonomy_chain::types::LightServerStatus> {
        let backend = self.backend.read().await;
        backend.light_server_status().await
    }

    pub async fn export_mnemonic(&self, passphrase: &str) -> Result<String> {
        self.keystore.unlocked_mnemonic(self.coin, passphrase).await
    }

    pub async fn watch_scripthashes(&self) -> Result<Vec<String>> {
        use vericonomy_chain::electrum::scripthash::scripthash_from_script_hex;

        if !self.keystore.is_unlocked(self.coin).await? {
            return Ok(Vec::new());
        }
        let mut scripts = self.keystore.funded_script_hexes(self.coin).await?;
        for script in self.keystore.cached_script_hexes(self.coin).await? {
            if !scripts.iter().any(|s| s == &script) {
                scripts.push(script);
            }
        }
        scripts.sort();
        scripts.dedup();
        let mut out = Vec::with_capacity(scripts.len());
        for script_hex in scripts {
            out.push(scripthash_from_script_hex(&script_hex)?);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}
