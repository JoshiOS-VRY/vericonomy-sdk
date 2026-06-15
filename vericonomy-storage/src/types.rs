//! Shared wallet state types for storage and facade layers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vericonomy_chain_params::CoinId;

/// Opaque encrypted secret material (mnemonic, xprv, or passphrase-protected blob).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretBlob {
    pub ciphertext: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightWalletRecord {
    pub coin: String,
    pub encrypted_mnemonic: String,
    pub salt: String,
    pub nonce: String,
    pub created_at: u64,
    pub next_receive_index: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub cached_script_hexes: Vec<String>,
    #[serde(default)]
    pub addresses_scan_complete: bool,
    #[serde(default)]
    pub funded_script_hexes: Vec<String>,
    #[serde(default)]
    pub index_precache_offset: u32,
    #[serde(default)]
    pub index_gap_external: u32,
    #[serde(default)]
    pub index_gap_external_done: bool,
    #[serde(default)]
    pub index_gap_internal: u32,
    #[serde(default)]
    pub index_gap_internal_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LightKeystore {
    pub wallets: HashMap<String, LightWalletRecord>,
    #[serde(default)]
    pub unlocked_until_by_coin: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingProgress {
    pub precache_offset: u32,
    pub gap_external: u32,
    pub gap_external_done: bool,
    pub gap_internal: u32,
    pub gap_internal_done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    Idle,
    Indexing,
    Syncing,
    Ready,
}

impl Default for ScanPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletStatus {
    pub scan_phase: ScanPhase,
    pub scan_complete: bool,
    pub sync_in_flight: bool,
    pub balance_ready: bool,
    pub indexing: IndexingProgress,
    pub unlocked: bool,
    pub wallet_exists: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub scan_complete: bool,
    pub balance_confirmed_sats: i64,
    pub balance_unconfirmed_sats: i64,
}

pub fn wallet_id(coin: CoinId) -> String {
    coin.as_str().to_string()
}
