//! Electrum connection manager with failover across configured servers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::connection::{ElectrumConnection, ElectrumServerEndpoint};
use super::history::{enrich_wallet_history, HistoryTxFetcher};
use super::indexing::{
    BURST_PAUSE_MS, BURST_SIZE, REFRESH_BURST_PAUSE_MS, REFRESH_BURST_SIZE,
};
use super::scripthash::scripthash_from_script_hex;
use super::throttle;
use crate::types::*;
use crate::ChainBackend;
use vericonomy_chain_params::{CoinId, CoinTarget};
use vericonomy_errors::{Result, WalletError};

const COIN_SATS: f64 = 100_000_000.0;
const RATE_LIMIT_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RATE_LIMIT_BACKOFF_MAX: Duration = Duration::from_secs(30);
const RATE_LIMIT_RETRIES: u32 = 6;

/// ElectrumX returns `{ height, hex }`; older servers may use `[hex, height]`.
fn parse_headers_subscribe(value: &Value) -> Option<(u32, String)> {
    if let Some(obj) = value.as_object() {
        let height = obj.get("height").and_then(Value::as_u64)? as u32;
        let hash = obj
            .get("hex")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Some((height, hash));
    }
    let arr = value.as_array()?;
    let hash = arr.first().and_then(Value::as_str).unwrap_or_default().to_string();
    let height = arr.get(1).and_then(Value::as_u64)? as u32;
    Some((height, hash))
}

#[derive(Default)]
struct IndexingLimits {
    active: bool,
    rpc_budget: u32,
    max_scripts_per_batch: u32,
    exhausted: bool,
}

pub struct ElectrumLightClient {
    coin: CoinId,
    servers: Vec<ElectrumServerEndpoint>,
    active_index: AtomicUsize,
    connection: tokio::sync::RwLock<Option<Arc<ElectrumConnection>>>,
    last_latency_ms: tokio::sync::RwLock<Option<u64>>,
    tip_height: tokio::sync::RwLock<Option<u32>>,
    consecutive_failures: AtomicUsize,
    indexing: tokio::sync::RwLock<IndexingLimits>,
}

impl ElectrumLightClient {
    pub fn new(coin: CoinId, server_uris: &[String]) -> Result<Self> {
        if server_uris.is_empty() {
            return Err(WalletError::other(format!(
                "no Electrum servers configured for {}",
                coin.as_str()
            )));
        }
        let servers: Result<Vec<_>> = server_uris.iter().map(|u| ElectrumServerEndpoint::parse(u)).collect();
        Ok(Self {
            coin,
            servers: servers?,
            active_index: AtomicUsize::new(0),
            connection: tokio::sync::RwLock::new(None),
            last_latency_ms: tokio::sync::RwLock::new(None),
            tip_height: tokio::sync::RwLock::new(None),
            consecutive_failures: AtomicUsize::new(0),
            indexing: tokio::sync::RwLock::new(IndexingLimits::default()),
        })
    }

    async fn take_rpc_budget(&self, count: u32) -> Result<()> {
        let mut state = self.indexing.write().await;
        if !state.active {
            return Ok(());
        }
        if state.exhausted || state.rpc_budget == 0 {
            state.exhausted = true;
            return Err(WalletError::other("indexing batch limit reached"));
        }
        if count > state.rpc_budget {
            state.exhausted = true;
            return Err(WalletError::other("indexing batch limit reached"));
        }
        state.rpc_budget -= count;
        if state.rpc_budget == 0 {
            state.exhausted = true;
        }
        Ok(())
    }

    async fn pacing(&self) -> (usize, Duration) {
        if self.indexing.read().await.active {
            (BURST_SIZE, Duration::from_millis(BURST_PAUSE_MS))
        } else {
            (REFRESH_BURST_SIZE, Duration::from_millis(REFRESH_BURST_PAUSE_MS))
        }
    }

    async fn max_scripts_per_batch(&self) -> usize {
        let state = self.indexing.read().await;
        if state.active && state.max_scripts_per_batch > 0 {
            state.max_scripts_per_batch as usize
        } else {
            usize::MAX
        }
    }

    pub fn default_servers(coin: CoinId, target: CoinTarget) -> Vec<String> {
        coin.profile().default_electrum_servers(target.network)
    }

    async fn ensure_connected(&self) -> Result<Arc<ElectrumConnection>> {
        {
            let guard = self.connection.read().await;
            if let Some(c) = guard.as_ref() {
                return Ok(c.clone());
            }
        }
        self.reconnect().await
    }

    async fn connect_server(&self, server_idx: usize) -> Result<Arc<ElectrumConnection>> {
        let ep = self
            .servers
            .get(server_idx)
            .ok_or_else(|| WalletError::other("no electrum servers"))?;
        let started = Instant::now();
        let conn = ElectrumConnection::connect(ep.clone()).await?;
        *self.last_latency_ms.write().await = Some(started.elapsed().as_millis() as u64);
        Ok(conn)
    }

    async fn reconnect(&self) -> Result<Arc<ElectrumConnection>> {
        let start_idx = self.active_index.load(Ordering::SeqCst);
        let n = self.servers.len();
        let mut last_err = WalletError::other("no electrum servers");

        for offset in 0..n {
            let idx = (start_idx + offset) % n;
            match self.connect_server(idx).await {
                Ok(conn) => {
                    self.active_index.store(idx, Ordering::SeqCst);
                    self.consecutive_failures.store(0, Ordering::SeqCst);
                    if let Ok(tip) = conn.call("blockchain.headers.subscribe", json!([])).await {
                        if let Some((height, _)) = parse_headers_subscribe(&tip) {
                            *self.tip_height.write().await = Some(height);
                        }
                    }
                    *self.connection.write().await = Some(conn.clone());
                    return Ok(conn);
                }
                Err(e) => {
                    tracing::warn!(
                        "electrum failover: {} failed: {e}",
                        self.servers[idx].display()
                    );
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    async fn call_primary(&self, method: &str, params: Value) -> Result<Value> {
        let conn = self.ensure_connected().await?;
        conn.call(method, params).await
    }

    async fn call_with_backoff(&self, method: &str, params: Value) -> Result<Value> {
        let mut backoff = RATE_LIMIT_BACKOFF_INITIAL;
        for attempt in 0..RATE_LIMIT_RETRIES {
            match self.call_primary(method, params.clone()).await {
                Ok(v) => {
                    self.consecutive_failures.store(0, Ordering::SeqCst);
                    return Ok(v);
                }
                Err(e) if e.is_electrum_rate_limited() => {
                    throttle::record_rate_limit(self.coin);
                    tracing::warn!(
                        "electrum rate limited on {} (attempt {}), backing off {:?}",
                        self.servers
                            .get(self.active_index.load(Ordering::SeqCst))
                            .map(|s| s.display())
                            .unwrap_or_default(),
                        attempt + 1,
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RATE_LIMIT_BACKOFF_MAX);
                }
                Err(e) if e.is_electrum_transport() => {
                    return self.failover_and_retry(method, params).await;
                }
                Err(e) => return Err(e),
            }
        }
        throttle::record_rate_limit(self.coin);
        Err(WalletError::other(format!(
            "electrum rate limited after {RATE_LIMIT_RETRIES} retries: {method}"
        )))
    }

    async fn failover_and_retry(&self, method: &str, params: Value) -> Result<Value> {
        let fails = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
        *self.connection.write().await = None;
        if fails >= 2 && self.servers.len() > 1 {
            let next = (self.active_index.load(Ordering::SeqCst) + 1) % self.servers.len();
            self.active_index.store(next, Ordering::SeqCst);
            self.consecutive_failures.store(0, Ordering::SeqCst);
            tracing::warn!(
                "electrum transport failure — failing over to {}",
                self.servers[next].display()
            );
        }
        let conn = self.reconnect().await?;
        conn.call(method, params).await
    }

    async fn call_with_failover(&self, method: &str, params: Value) -> Result<Value> {
        self.call_with_failover_inner(method, params, false).await
    }

    async fn call_with_failover_critical(&self, method: &str, params: Value) -> Result<Value> {
        self.call_with_failover_inner(method, params, true).await
    }

    async fn call_with_failover_inner(
        &self,
        method: &str,
        params: Value,
        bypass_cooldown: bool,
    ) -> Result<Value> {
        if !bypass_cooldown && throttle::is_in_cooldown(self.coin) {
            return Err(WalletError::other(
                "electrum temporarily paused after rate limit — try again in a few minutes",
            ));
        }
        match self.call_primary(method, params.clone()).await {
            Ok(v) => {
                self.consecutive_failures.store(0, Ordering::SeqCst);
                Ok(v)
            }
            Err(e) if e.is_electrum_rate_limited() => self.call_with_backoff(method, params).await,
            Err(e) if e.is_electrum_transport() => self.failover_and_retry(method, params).await,
            Err(e) => Err(e),
        }
    }

    async fn fetch_scripthash_json(
        &self,
        method: &str,
        hashes: &[String],
    ) -> Result<Vec<Value>> {
        let max_batch = self.max_scripts_per_batch().await;
        let (burst, pause) = self.pacing().await;
        let mut out = Vec::with_capacity(hashes.len());
        let mut rpcs_this_call = 0u32;
        for (i, sh) in hashes.iter().enumerate() {
            if max_batch != usize::MAX && i > 0 && i % max_batch == 0 {
                tokio::time::sleep(pause).await;
            }
            self.take_rpc_budget(1).await?;
            out.push(self.call_with_backoff(method, json!([sh])).await?);
            rpcs_this_call += 1;
            if rpcs_this_call as usize % burst == 0 && i + 1 < hashes.len() {
                tokio::time::sleep(pause).await;
            }
        }
        Ok(out)
    }

    async fn scripthashes(&self, script_hexes: &[String]) -> Result<Vec<String>> {
        script_hexes
            .iter()
            .map(|h| scripthash_from_script_hex(h))
            .collect()
    }

    async fn fetch_balances_json(&self, hashes: &[String]) -> Result<Vec<Value>> {
        self.fetch_scripthash_json("blockchain.scripthash.get_balance", hashes)
            .await
    }

    fn wallet_balance_from_json(bal: &Value) -> WalletBalance {
        WalletBalance {
            confirmed_sats: bal.get("confirmed").and_then(Value::as_i64).unwrap_or(0),
            unconfirmed_sats: bal.get("unconfirmed").and_then(Value::as_i64).unwrap_or(0),
            immature_sats: 0,
        }
    }
}

#[async_trait]
impl ChainBackend for ElectrumLightClient {
    fn backend_kind(&self) -> BackendKind {
        BackendKind::ElectrumLight
    }

    fn connection_status(&self) -> ConnectionStatus {
        if self.connection.try_read().map(|g| g.is_some()).unwrap_or(false) {
            ConnectionStatus::Connected
        } else {
            ConnectionStatus::Disconnected
        }
    }

    async fn get_tip(&self) -> Result<ChainTip> {
        let result = self
            .call_with_failover("blockchain.headers.subscribe", json!([]))
            .await?;
        let (height, hash) = parse_headers_subscribe(&result)
            .ok_or_else(|| WalletError::other("electrum tip missing height"))?;
        *self.tip_height.write().await = Some(height);
        Ok(ChainTip { height, hash })
    }

    async fn network_info(&self) -> Result<NetworkInfo> {
        let fee: f64 = self
            .call_with_failover("blockchain.relayfee", json!([]))
            .await
            .and_then(|v| v.as_f64().ok_or_else(|| WalletError::other("relayfee")))
            .unwrap_or(0.000_01);
        Ok(NetworkInfo {
            relay_fee_per_kb: fee * COIN_SATS / 1000.0,
        })
    }

    async fn get_balance_for_scripts(&self, script_hexes: &[String]) -> Result<WalletBalance> {
        if script_hexes.is_empty() {
            return Ok(WalletBalance {
                confirmed_sats: 0,
                unconfirmed_sats: 0,
                immature_sats: 0,
            });
        }
        let hashes = self.scripthashes(script_hexes).await?;
        let mut confirmed = 0i64;
        let mut unconfirmed = 0i64;
        for bal in self.fetch_balances_json(&hashes).await? {
            confirmed += bal.get("confirmed").and_then(Value::as_i64).unwrap_or(0);
            unconfirmed += bal.get("unconfirmed").and_then(Value::as_i64).unwrap_or(0);
        }
        Ok(WalletBalance {
            confirmed_sats: confirmed,
            unconfirmed_sats: unconfirmed,
            immature_sats: 0,
        })
    }

    async fn get_balances_per_script(&self, script_hexes: &[String]) -> Result<Vec<WalletBalance>> {
        if script_hexes.is_empty() {
            return Ok(Vec::new());
        }
        let hashes = self.scripthashes(script_hexes).await?;
        Ok(self
            .fetch_balances_json(&hashes)
            .await?
            .iter()
            .map(Self::wallet_balance_from_json)
            .collect())
    }

    async fn list_utxos_for_scripts(&self, script_hexes: &[String]) -> Result<Vec<Utxo>> {
        if script_hexes.is_empty() {
            return Ok(Vec::new());
        }
        let tip = self.get_tip().await.ok();
        let hashes = self.scripthashes(script_hexes).await?;
        let results = self
            .fetch_scripthash_json("blockchain.scripthash.listunspent", &hashes)
            .await?;
        let mut out = Vec::new();
        for (items, script_hex) in results.iter().zip(script_hexes.iter()) {
            let items = items.as_array().cloned().unwrap_or_default();
            for item in items {
                let txid = item
                    .get("tx_hash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let vout = item.get("tx_pos").and_then(Value::as_u64).unwrap_or(0) as u32;
                let value_sats = item.get("value").and_then(Value::as_u64).unwrap_or(0) as i64;
                let height = item.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
                let conf = tip
                    .as_ref()
                    .map(|t| t.height.saturating_sub(height) + 1)
                    .unwrap_or(0);
                out.push(Utxo {
                    txid,
                    vout,
                    value_sats,
                    height,
                    address: String::new(),
                    script_hex: script_hex.clone(),
                    confirmations: conf,
                });
            }
        }
        Ok(out)
    }

    async fn get_history_for_scripts(
        &self,
        script_hexes: &[String],
        limit: usize,
    ) -> Result<Vec<WalletTx>> {
        let tip = self.get_tip().await.ok();
        let hashes = self.scripthashes(script_hexes).await?;
        let history_json = self
            .fetch_scripthash_json("blockchain.scripthash.get_history", &hashes)
            .await?;
        let mut merged: Vec<WalletTx> = Vec::new();
        for items in history_json {
            let items = items.as_array().cloned().unwrap_or_default();
            for item in items {
                let txid = item
                    .get("tx_hash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let height = item.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
                let fee = item.get("fee").and_then(Value::as_i64);
                let conf = if height <= 0 {
                    0
                } else {
                    tip.as_ref()
                        .map(|t| (t.height as i32 - height + 1).max(0))
                        .unwrap_or(0)
                };
                merged.push(WalletTx {
                    txid,
                    height,
                    fee_sats: fee,
                    category: if height <= 0 {
                        "unconfirmed".into()
                    } else {
                        "receive".into()
                    },
                    amount: 0.0,
                    address: None,
                    confirmations: conf,
                    time: None,
                    blockhash: None,
                    blockheight: if height > 0 { Some(height as u32) } else { None },
                });
            }
        }
        merged.sort_by(|a, b| b.height.cmp(&a.height));
        merged.dedup_by(|a, b| a.txid == b.txid);
        merged.truncate(limit);
        Ok(merged)
    }

    async fn enrich_tx_history_batch(
        &self,
        coin: CoinId,
        script_hexes: &[String],
        txs: &mut [WalletTx],
        max_rows: usize,
    ) -> Result<()> {
        enrich_wallet_history(coin, script_hexes, txs, self, Some(max_rows)).await
    }

    async fn get_raw_tx_hex(&self, txid: &str) -> Result<String> {
        let txid = txid.trim();
        // Electrum spec: verbose=false returns raw tx as hex; verbose=true returns JSON.
        let raw = self
            .call_with_failover_critical("blockchain.transaction.get", json!([txid, false]))
            .await?;
        match crate::tx_hex::parse_electrum_transaction_get(&raw) {
            Ok(hex) => Ok(hex),
            Err(first) => {
                let verbose = self
                    .call_with_failover_critical("blockchain.transaction.get", json!([txid, true]))
                    .await?;
                crate::tx_hex::parse_electrum_transaction_get(&verbose)
                    .map_err(|_| first)
            }
        }
    }

    async fn estimate_fee(&self, target_blocks: u32) -> Result<FeeRate> {
        let from_estimate = self
            .call_with_failover(
                "blockchain.estimatefee",
                json!([target_blocks.max(1)]),
            )
            .await
            .ok()
            .and_then(|v| v.as_f64())
            .filter(|fee| *fee >= 0.0);
        let coins_per_kb = if let Some(fee) = from_estimate {
            fee
        } else {
            self.call_with_failover("blockchain.relayfee", json!([]))
                .await
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|fee| *fee >= 0.0)
                .unwrap_or(0.0001)
        };
        Ok(FeeRate { coins_per_kb })
    }

    async fn broadcast_tx(&self, raw_hex: &str) -> Result<String> {
        let txid = self
            .call_with_failover_critical("blockchain.transaction.broadcast", json!([raw_hex]))
            .await?;
        Ok(txid
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| txid.to_string()))
    }

    async fn set_initial_indexing_limits(
        &self,
        max_scripthash_rpcs: u32,
        max_scripts_per_batch: u32,
    ) {
        let mut state = self.indexing.write().await;
        *state = IndexingLimits {
            active: true,
            rpc_budget: max_scripthash_rpcs,
            max_scripts_per_batch,
            exhausted: false,
        };
    }

    fn indexing_budget_exhausted(&self) -> bool {
        self.indexing
            .try_read()
            .map(|s| s.exhausted)
            .unwrap_or(false)
    }

    async fn clear_initial_indexing_limits(&self) {
        *self.indexing.write().await = IndexingLimits::default();
    }

    async fn light_server_status(&self) -> Option<LightServerStatus> {
        let idx = self.active_index.load(Ordering::SeqCst);
        let ep = self.servers.get(idx)?;
        let active_idx = self.active_index.load(Ordering::SeqCst);
        let active_ep = self.servers.get(active_idx).unwrap_or(ep);
        let cached_tip = *self.tip_height.read().await;
        let has_connection = self
            .connection
            .try_read()
            .map(|g| g.is_some())
            .unwrap_or(false);

        let latency_ms = *self.last_latency_ms.read().await;
        let build_status = |connected: bool,
                            banner_text: Option<String>,
                            tip_height: Option<u32>,
                            latency_ms: Option<u64>| LightServerStatus {
            connected,
            server_host: Some(active_ep.host.clone()),
            server_port: Some(active_ep.port),
            latency_ms,
            tip_height,
            banner: banner_text,
            failover_index: active_idx,
            servers_total: self.servers.len(),
        };

        // During rate-limit cooldown, return cached status without new RPCs.
        if throttle::is_in_cooldown(self.coin) {
            return Some(build_status(
                has_connection && cached_tip.is_some(),
                None,
                cached_tip,
                latency_ms,
            ));
        }

        // Reuse cached tip between probes — UI polls every ~45s; subscribe every call
        // was tripping server-side excessive-usage limits.
        if has_connection && cached_tip.is_some() && !throttle::status_probe_due(self.coin) {
            let banner_text = if let Some(conn) = self.connection.read().await.as_ref() {
                conn.banner().await
            } else {
                None
            };
            return Some(build_status(true, banner_text, cached_tip, latency_ms));
        }

        throttle::mark_status_probe(self.coin);

        let connected = match self.ensure_connected().await {
            Ok(conn) => {
                if let Ok(tip) = conn.call("blockchain.headers.subscribe", json!([])).await {
                    if let Some((height, _)) = parse_headers_subscribe(&tip) {
                        *self.tip_height.write().await = Some(height);
                    }
                }
                true
            }
            Err(e) => {
                tracing::warn!("electrum status check failed for {}: {e}", ep.display());
                cached_tip.is_some()
            }
        };
        let banner_text = if let Some(conn) = self.connection.read().await.as_ref() {
            conn.banner().await
        } else {
            None
        };
        let tip_height = *self.tip_height.read().await;
        let latency_ms = *self.last_latency_ms.read().await;
        Some(build_status(connected, banner_text, tip_height, latency_ms))
    }
}

#[async_trait]
impl HistoryTxFetcher for ElectrumLightClient {
    async fn fetch_raw_tx_hex(&self, txid: &str) -> Result<String> {
        self.get_raw_tx_hex(txid).await
    }
}
