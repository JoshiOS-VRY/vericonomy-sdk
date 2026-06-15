//! Light-wallet sync engine (gap scan, UTXO refresh, history).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use vericonomy_chain::electrum::indexing::{RPC_BUDGET_PER_SYNC, SCRIPTS_PER_BATCH};
use vericonomy_chain::ChainBackend;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;
use vericonomy_storage::{
    LightKeystoreService, ScanPhase, SyncReport, TxCache, UtxoCache, WalletStatus,
};

use crate::gap_scan::{discover_script_hexes, enrich_utxo_addresses, GapScanHook, GAP_LIMIT};
use crate::history::{merge_tx_history, HistorySource, NoopHistorySource};
use crate::wallet_cache::{
    balance_from_utxo_cache, store_last_electrum_utxo_keys, TX_HISTORY_CACHE_LIMIT,
};

const SYNC_TIMEOUT: Duration = Duration::from_secs(300);
const PRECACHE_PROBE_COMPLETE_META: &str = "precache_probe_complete";
const LAST_MANUAL_RESCAN_META: &str = "last_manual_rescan_at";
const MANUAL_RESCAN_COOLDOWN_SECS: u64 = 3600;

struct SyncInFlight(Mutex<HashSet<String>>);

impl SyncInFlight {
    fn new() -> Self {
        Self(Mutex::new(HashSet::new()))
    }

    async fn try_begin(&self, coin: CoinId) -> bool {
        self.0.lock().await.insert(coin.as_str().to_string())
    }

    async fn end(&self, coin: CoinId) {
        self.0.lock().await.remove(coin.as_str());
    }

    async fn contains(&self, coin: CoinId) -> bool {
        self.0.lock().await.contains(coin.as_str())
    }
}

pub struct LightSyncEngine<K: vericonomy_storage::KeystoreStore, U, T> {
    keystore: Arc<LightKeystoreService<K>>,
    utxo_cache: Arc<U>,
    tx_cache: Arc<T>,
    in_flight: SyncInFlight,
}

impl<K, U, T> LightSyncEngine<K, U, T>
where
    K: vericonomy_storage::KeystoreStore + Send + Sync,
    U: UtxoCache + Send + Sync,
    T: TxCache + Send + Sync,
{
    pub fn new(
        keystore: Arc<LightKeystoreService<K>>,
        utxo_cache: Arc<U>,
        tx_cache: Arc<T>,
    ) -> Self {
        Self {
            keystore,
            utxo_cache,
            tx_cache,
            in_flight: SyncInFlight::new(),
        }
    }

    pub async fn sync_in_flight(&self, coin: CoinId) -> bool {
        self.in_flight.contains(coin).await
    }

    pub async fn wallet_status<B: ChainBackend>(
        &self,
        coin: CoinId,
        _backend: &B,
    ) -> Result<WalletStatus> {
        let exists = self.keystore.wallet_exists(coin).await?;
        let unlocked = self.keystore.is_unlocked(coin).await?;
        let scan_complete = !self.keystore.needs_full_address_scan(coin).await?;
        let indexing = self.keystore.indexing_progress(coin).await?;
        let sync_in_flight = self.sync_in_flight(coin).await;
        let balance = balance_from_utxo_cache(self.utxo_cache.as_ref(), coin).await?;
        let balance_ready = scan_complete || balance.total_sats() > 0;
        let scan_phase = if sync_in_flight {
            ScanPhase::Syncing
        } else if !scan_complete {
            ScanPhase::Indexing
        } else if balance_ready {
            ScanPhase::Ready
        } else {
            ScanPhase::Idle
        };
        Ok(WalletStatus {
            scan_phase,
            scan_complete,
            sync_in_flight,
            balance_ready,
            indexing,
            unlocked,
            wallet_exists: exists,
        })
    }

    pub async fn sync<B: ChainBackend, H: HistorySource>(
        &self,
        coin: CoinId,
        backend: &B,
        history: &H,
    ) -> Result<SyncReport> {
        if !self.in_flight.try_begin(coin).await {
            let balance = balance_from_utxo_cache(self.utxo_cache.as_ref(), coin).await?;
            return Ok(SyncReport {
                scan_complete: !self.keystore.needs_full_address_scan(coin).await?,
                balance_confirmed_sats: balance.confirmed_sats,
                balance_unconfirmed_sats: balance.unconfirmed_sats,
            });
        }
        let result = tokio::time::timeout(
            SYNC_TIMEOUT,
            self.sync_inner(coin, backend, history),
        )
        .await;
        self.in_flight.end(coin).await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err(vericonomy_errors::WalletError::other(format!(
                "light wallet sync timed out after {}s",
                SYNC_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn sync_inner<B: ChainBackend, H: HistorySource>(
        &self,
        coin: CoinId,
        backend: &B,
        history: &H,
    ) -> Result<SyncReport> {
        if !self.keystore.wallet_exists(coin).await? {
            return Ok(SyncReport::default());
        }
        let needs_scan = self.keystore.needs_full_address_scan(coin).await?;
        if needs_scan {
            if !self.keystore.is_unlocked(coin).await?
                || !self.keystore.signing_session_active(coin)
            {
                return Ok(SyncReport::default());
            }
            let phrase = self.keystore.unlocked_mnemonic(coin, "").await?;
            self.run_gap_scan(coin, &phrase, backend).await?;
        } else {
            let phrase = if self.keystore.is_unlocked(coin).await?
                && self.keystore.signing_session_active(coin)
            {
                Some(self.keystore.unlocked_mnemonic(coin, "").await?)
            } else {
                None
            };
            self.refresh_utxos_from_network(coin, phrase.as_deref(), backend)
                .await?;
            let funded = self.keystore.funded_script_hexes(coin).await?;
            self.refresh_tx_history(coin, &funded, backend, history, true)
                .await;
        }
        let balance = balance_from_utxo_cache(self.utxo_cache.as_ref(), coin).await?;
        Ok(SyncReport {
            scan_complete: !self.keystore.needs_full_address_scan(coin).await?,
            balance_confirmed_sats: balance.confirmed_sats,
            balance_unconfirmed_sats: balance.unconfirmed_sats,
        })
    }

    pub async fn refresh_balance<B: ChainBackend>(
        &self,
        coin: CoinId,
        backend: &B,
    ) -> Result<()> {
        if !self.keystore.wallet_exists(coin).await? || !self.keystore.is_unlocked(coin).await? {
            return Ok(());
        }
        let phrase = if self.keystore.signing_session_active(coin) {
            Some(self.keystore.unlocked_mnemonic(coin, "").await?)
        } else {
            None
        };
        self.refresh_utxos_from_network(coin, phrase.as_deref(), backend)
            .await?;
        Ok(())
    }

    pub async fn refresh_pending<B: ChainBackend, H: HistorySource>(
        &self,
        coin: CoinId,
        backend: &B,
        history: &H,
    ) -> Result<()> {
        let funded = self.keystore.funded_script_hexes(coin).await?;
        self.refresh_tx_history(coin, &funded, backend, history, true)
            .await;
        Ok(())
    }

    pub async fn begin_manual_rescan<C: UtxoCache>(
        &self,
        coin: CoinId,
        utxo_cache: &C,
    ) -> Result<()> {
        if !self.keystore.is_unlocked(coin).await? {
            return Err(vericonomy_errors::WalletError::LockedWallet {
                code: vericonomy_errors::WalletError::CODE_LOCKED_WALLET,
            });
        }
        if self.in_flight.contains(coin).await {
            return Err(vericonomy_errors::WalletError::other(
                "address rescan is already running — wait for it to finish",
            ));
        }
        if self.keystore.needs_full_address_scan(coin).await?
            && !utxo_cache
                .get_meta(coin, LAST_MANUAL_RESCAN_META)
                .await?
                .is_some()
        {
            return Err(vericonomy_errors::WalletError::other(
                "initial address scan still running — wait for it to finish",
            ));
        }
        let remaining = manual_rescan_cooldown_remaining_secs(utxo_cache, coin).await;
        if remaining > 0 {
            let minutes = (remaining + 59) / 60;
            return Err(vericonomy_errors::WalletError::other(format!(
                "rescan was used recently — try again in about {minutes} minute{}",
                if minutes == 1 { "" } else { "s" }
            )));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        utxo_cache
            .set_meta(coin, LAST_MANUAL_RESCAN_META, &now)
            .await?;
        self.keystore.mark_address_scan_incomplete(coin).await?;
        let _ = utxo_cache
            .set_meta(coin, PRECACHE_PROBE_COMPLETE_META, "0")
            .await;
        let _ = utxo_cache
            .set_meta(coin, "precache_probe_offset", "0")
            .await;
        Ok(())
    }

    pub async fn list_spendable_utxos<B: ChainBackend>(
        &self,
        coin: CoinId,
        backend: &B,
        passphrase: &str,
    ) -> Result<Vec<vericonomy_chain::types::Utxo>> {
        let phrase = self.keystore.unlocked_mnemonic(coin, passphrase).await?;
        self.refresh_utxos_from_network(coin, Some(&phrase), backend)
            .await?;
        let mut utxos = self.utxo_cache.list_utxos(coin).await?;
        enrich_utxo_addresses(coin, &phrase, None, &mut utxos)?;
        utxos.retain(|u| u.height > 0);
        Ok(utxos)
    }

    pub async fn list_transactions<B: ChainBackend, H: HistorySource>(
        &self,
        coin: CoinId,
        backend: &B,
        history: &H,
        limit: usize,
    ) -> Result<Vec<vericonomy_chain::types::WalletTx>> {
        let funded = self.keystore.funded_script_hexes(coin).await?;
        self.refresh_tx_history(coin, &funded, backend, history, true)
            .await;
        self.tx_cache.load_history(coin, limit).await
    }

    async fn refresh_utxos_from_network<B: ChainBackend>(
        &self,
        coin: CoinId,
        phrase: Option<&str>,
        backend: &B,
    ) -> Result<()> {
        let mut funded = self.keystore.funded_script_hexes(coin).await?;
        self.extend_funded_from_precached_balances(coin, phrase, backend, &mut funded)
            .await?;
        if funded.is_empty() {
            return Ok(());
        }
        self.refresh_utxos(coin, &funded, backend, phrase).await?;
        Ok(())
    }

    async fn extend_funded_from_precached_balances<B: ChainBackend>(
        &self,
        coin: CoinId,
        phrase: Option<&str>,
        backend: &B,
        funded: &mut Vec<String>,
    ) -> Result<()> {
        let precached = self.keystore.cached_script_hexes(coin).await?;
        if precached.is_empty() {
            return Ok(());
        }
        let chunk_size = 32.min(precached.len());
        let chunk = &precached[..chunk_size];
        let balances = backend.get_balances_per_script(chunk).await?;
        for (script, bal) in chunk.iter().zip(balances) {
            if bal.total_sats() > 0 && !funded.iter().any(|s| s == script) {
                funded.push(script.clone());
            }
        }
        if phrase.is_some() {
            self.keystore.set_funded_script_hexes(coin, funded).await?;
        }
        Ok(())
    }

    async fn refresh_utxos<B: ChainBackend>(
        &self,
        coin: CoinId,
        funded_scripts: &[String],
        backend: &B,
        phrase: Option<&str>,
    ) -> Result<()> {
        let mut utxos = backend.list_utxos_for_scripts(funded_scripts).await?;
        store_last_electrum_utxo_keys(self.utxo_cache.as_ref(), coin, &utxos).await?;
        if let Some(phrase) = phrase {
            enrich_utxo_addresses(coin, phrase, None, &mut utxos)?;
        }
        let _ = self
            .utxo_cache
            .replace_utxos(coin, &utxos)
            .await?;
        if let Ok(tip) = backend.get_tip().await {
            let _ = self
                .utxo_cache
                .set_meta(coin, "tip_height", &tip.height.to_string())
                .await;
        }
        for utxo in &utxos {
            if !utxo.script_hex.is_empty() {
                let _ = self
                    .keystore
                    .register_funded_script_hex(coin, &utxo.script_hex)
                    .await;
            }
        }
        Ok(())
    }

    async fn run_gap_scan<B: ChainBackend>(&self, coin: CoinId, phrase: &str, backend: &B) -> Result<()> {
        backend
            .set_initial_indexing_limits(RPC_BUDGET_PER_SYNC, SCRIPTS_PER_BATCH)
            .await;
        let result = self.run_gap_scan_inner(coin, phrase, backend).await;
        backend.clear_initial_indexing_limits().await;
        result
    }

    async fn run_gap_scan_inner<B: ChainBackend>(
        &self,
        coin: CoinId,
        phrase: &str,
        backend: &B,
    ) -> Result<()> {
        let mut progress = self.keystore.indexing_progress(coin).await?;
        let mut funded = self.keystore.funded_script_hexes(coin).await?;

        struct Hook<'a, B, K, U, T>
        where
            K: vericonomy_storage::KeystoreStore,
        {
            engine: &'a LightSyncEngine<K, U, T>,
            coin: CoinId,
            phrase: &'a str,
            backend: &'a B,
        }

        #[async_trait]
        impl<'a, B, K, U, T> GapScanHook for Hook<'a, B, K, U, T>
        where
            B: ChainBackend,
            K: vericonomy_storage::KeystoreStore + Send + Sync,
            U: UtxoCache + Send + Sync,
            T: TxCache + Send + Sync,
        {
            async fn on_funded_batch(&self, funded: &[String], utxo_refresh: bool) -> Result<()> {
                self.engine.keystore.set_funded_script_hexes(self.coin, funded).await?;
                if utxo_refresh {
                    self.engine
                        .refresh_utxos(self.coin, funded, self.backend, Some(self.phrase))
                        .await?;
                }
                Ok(())
            }
        }

        let hook = Hook {
            engine: self,
            coin,
            phrase,
            backend,
        };

        const MAX_SLICES: usize = 12;
        let mut scan_complete = false;
        for _ in 0..MAX_SLICES {
            backend
                .set_initial_indexing_limits(RPC_BUDGET_PER_SYNC, SCRIPTS_PER_BATCH)
                .await;
            let progress_before = progress;
            let (_, done) = discover_script_hexes(
                coin,
                phrase,
                None,
                GAP_LIMIT,
                &mut progress,
                backend,
                SCRIPTS_PER_BATCH,
                &mut funded,
                Some(&hook),
                false,
            )
            .await?;
            self.keystore.set_indexing_progress(coin, progress).await?;
            scan_complete = done;
            if scan_complete {
                break;
            }
            if progress == progress_before {
                break;
            }
        }

        if scan_complete {
            self.keystore.mark_address_scan_complete(coin).await?;
            if !funded.is_empty() {
                if let Ok(max_idx) = crate::gap_scan::max_external_receive_index_for_scripts(
                    coin,
                    phrase,
                    None,
                    &funded,
                ) {
                    let _ = self
                        .keystore
                        .set_receive_index_at_least(coin, max_idx.saturating_add(1))
                        .await;
                }
            }
            if funded.is_empty() {
                let _ = self.utxo_cache.replace_utxos(coin, &[]).await;
                let _ = self
                    .utxo_cache
                    .set_meta(coin, PRECACHE_PROBE_COMPLETE_META, "1")
                    .await;
            } else {
                self.refresh_utxos(coin, &funded, backend, Some(phrase))
                    .await?;
            }
        }
        Ok(())
    }

    async fn refresh_tx_history<B: ChainBackend, H: HistorySource>(
        &self,
        coin: CoinId,
        funded: &[String],
        backend: &B,
        history: &H,
        merge_pending: bool,
    ) {
        if funded.is_empty() {
            return;
        }
        let tip = backend.get_tip().await.ok().map(|t| t.height);
        if let Some(height) = tip {
            let _ = self
                .utxo_cache
                .set_meta(coin, "tip_height", &height.to_string())
                .await;
        }
        let addresses = self.addresses_for_history(coin).await.unwrap_or_default();
        if let Ok(rows) = history
            .fetch_wallet_history(coin, &addresses, TX_HISTORY_CACHE_LIMIT, tip)
            .await
        {
            if !rows.is_empty() {
                let merged = if merge_pending {
                    merge_pending_electrum(coin, funded, backend, rows).await
                } else {
                    Some(rows)
                };
                if let Some(merged) = merged {
                    let _ = self.tx_cache.replace_history(coin, &merged).await;
                }
                return;
            }
        }
        if merge_pending {
            let base = self
                .tx_cache
                .load_history(coin, TX_HISTORY_CACHE_LIMIT)
                .await
                .unwrap_or_default();
            if let Some(merged) = merge_pending_electrum(coin, funded, backend, base).await {
                let _ = self.tx_cache.replace_history(coin, &merged).await;
            }
        }
    }

    async fn scripts_for_history(&self, coin: CoinId) -> Result<Vec<String>> {
        let funded = self.keystore.funded_script_hexes(coin).await?;
        if !funded.is_empty() {
            return Ok(funded);
        }
        self.keystore.cached_script_hexes(coin).await
    }

    /// P2PKH addresses for scripts the wallet has used (for explorer-indexed history).
    async fn addresses_for_history(&self, coin: CoinId) -> Result<Vec<String>> {
        use std::collections::HashSet;

        let scripts = self.scripts_for_history(coin).await?;
        if scripts.is_empty() {
            return Ok(Vec::new());
        }

        let mut utxo_addrs = HashSet::new();
        for utxo in self.utxo_cache.list_utxos(coin).await.unwrap_or_default() {
            if !utxo.address.is_empty() {
                utxo_addrs.insert(utxo.address.trim().to_string());
            }
        }

        let mut priority = Vec::new();
        let mut rest = Vec::new();
        let mut seen = HashSet::new();

        if self.keystore.is_unlocked(coin).await? && self.keystore.signing_session_active(coin) {
            let phrase = self.keystore.unlocked_mnemonic(coin, "").await?;
            let script_refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
            let map = crate::gap_scan::resolve_addresses_for_script_hexes(
                coin,
                &phrase,
                None,
                &script_refs,
            )?;
            for addr in map.values() {
                let clean = addr.trim();
                if clean.is_empty() || !seen.insert(clean.to_string()) {
                    continue;
                }
                if utxo_addrs.contains(clean) {
                    priority.push(clean.to_string());
                } else {
                    rest.push(clean.to_string());
                }
            }
        }

        for addr in utxo_addrs {
            if seen.insert(addr.clone()) {
                priority.push(addr);
            }
        }

        rest.sort();
        priority.extend(rest);
        Ok(priority)
    }
}

async fn merge_pending_electrum<B: ChainBackend>(
    coin: CoinId,
    funded: &[String],
    backend: &B,
    base: Vec<vericonomy_chain::types::WalletTx>,
) -> Option<Vec<vericonomy_chain::types::WalletTx>> {
    let mut history = backend.get_history_for_scripts(funded, 128).await.ok()?;
    history.retain(|tx| tx.height <= 0);
    if history.is_empty() {
        return Some(base);
    }
    let tip = backend.get_tip().await.ok().map(|t| t.height);
    struct Fetcher<'a, B>(&'a B);
    #[async_trait]
    impl<'a, B: ChainBackend> vericonomy_chain::electrum::history::HistoryTxFetcher for Fetcher<'a, B> {
        async fn fetch_raw_tx_hex(&self, txid: &str) -> Result<String> {
            self.0.get_raw_tx_hex(txid).await
        }
    }
    let fetcher = Fetcher(backend);
    let expanded = vericonomy_chain::electrum::history::expand_wallet_history_rows(
        coin,
        funded,
        &history,
        &fetcher,
        Some(64),
        tip,
    )
    .await
    .ok()?;
    Some(merge_tx_history(base, &expanded))
}

pub type DefaultHistory = NoopHistorySource;

async fn manual_rescan_cooldown_remaining_secs<C: UtxoCache>(
    cache: &C,
    coin: CoinId,
) -> u64 {
    let Some(raw) = cache.get_meta(coin, LAST_MANUAL_RESCAN_META).await.ok().flatten() else {
        return 0;
    };
    let Ok(last_at) = raw.parse::<u64>() else {
        return 0;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(last_at);
    MANUAL_RESCAN_COOLDOWN_SECS.saturating_sub(elapsed)
}
