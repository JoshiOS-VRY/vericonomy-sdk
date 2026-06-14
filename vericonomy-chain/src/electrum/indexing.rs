//! Rate limits for initial light-wallet address indexing (Electrum gap scan).
//!
//! Tuned for high-capacity Vericonomy Electrum (vrm3/vrc3) infrastructure.

/// Max scripthash RPC calls per `sync_light_wallet` indexing slice.
pub const RPC_BUDGET_PER_SYNC: u32 = 200;
/// Addresses queried per Electrum batch during indexing.
pub const SCRIPTS_PER_BATCH: u32 = 32;
/// Scripthash calls before a pacing pause (indexing mode).
pub const BURST_SIZE: usize = 24;
pub const BURST_PAUSE_MS: u64 = 40;

/// Steady-state refresh (known funded scripts only).
pub const REFRESH_BURST_SIZE: usize = 48;
pub const REFRESH_BURST_PAUSE_MS: u64 = 25;
