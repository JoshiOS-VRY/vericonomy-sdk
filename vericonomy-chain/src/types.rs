//! Shared chain/wallet types for full-node and light backends.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    FullNode,
    ElectrumLight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Degraded { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainTip {
    pub height: u32,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub relay_fee_per_kb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalance {
    pub confirmed_sats: i64,
    pub unconfirmed_sats: i64,
    pub immature_sats: i64,
}

impl WalletBalance {
    pub fn total_sats(&self) -> i64 {
        self.confirmed_sats + self.unconfirmed_sats + self.immature_sats
    }

    pub fn spendable_sats(&self) -> i64 {
        self.confirmed_sats + self.unconfirmed_sats
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value_sats: i64,
    pub height: u32,
    pub address: String,
    pub script_hex: String,
    pub confirmations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletTx {
    pub txid: String,
    pub height: i32,
    pub fee_sats: Option<i64>,
    pub category: String,
    pub amount: f64,
    pub address: Option<String>,
    pub confirmations: i32,
    pub time: Option<u64>,
    pub blockhash: Option<String>,
    pub blockheight: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FeeRate {
    /// Fee rate in coin units per kB (matches daemon `feeRate` convention).
    pub coins_per_kb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightServerStatus {
    pub connected: bool,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub latency_ms: Option<u64>,
    pub tip_height: Option<u32>,
    pub banner: Option<String>,
    pub failover_index: usize,
    pub servers_total: usize,
}
