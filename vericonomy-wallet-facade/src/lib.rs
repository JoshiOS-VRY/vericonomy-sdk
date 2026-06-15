//! High-level light-wallet facade consumed by mobile shells and Tauri.
//!
//! Low-level crypto lives in `vericonomy-wallet-core`, `vericonomy-hd`, and
//! `vericonomy-wallet-engine`. Chain I/O lives in `vericonomy-chain`. This crate
//! wires those pieces into portable flows (send, verify, balance helpers) so
//! platform code only handles storage, UI, and lifecycle.

pub mod balance;
pub mod explorer_history;
pub mod gap_scan;
pub mod history;
pub mod send;
pub mod session;
pub mod sync_engine;
pub mod utxo_prep;
pub mod verify;
pub mod wallet;
pub mod wallet_cache;

pub use balance::{balance_from_utxos, spendable_utxos};
pub use gap_scan::{
    discover_script_hexes, enrich_utxo_addresses, resolve_addresses_for_script_hexes, GAP_LIMIT,
};
pub use explorer_history::ExplorerHistorySource;
pub use history::{merge_tx_history, HistorySource, NoopHistorySource};
pub use send::{send_payment, SendPaymentParams, SendResult};
pub use session::LightWalletSession;
pub use sync_engine::LightSyncEngine;
pub use utxo_prep::prepare_utxos_for_signing;
pub use verify::{
    script_pays_to, utxo_script_matches_parent_tx, verify_send_outputs, verify_signed_p2pkh_inputs,
};
pub use wallet::LightWallet;
pub use wallet_cache::{
    apply_local_send_cache_update, balance_from_utxo_cache, TX_HISTORY_CACHE_LIMIT,
};

pub use vericonomy_chain::electrum::ElectrumLightClient;
pub use vericonomy_chain::ChainBackend;
pub use vericonomy_chain::types::{LightServerStatus, Utxo, WalletBalance, WalletTx};
pub use vericonomy_chain_params::CoinId;
pub use vericonomy_errors::{Result, WalletError};
pub use vericonomy_storage::{
    IndexingProgress, LightKeystoreService, ScanPhase, SyncReport, TxCache, UtxoCache,
    WalletIndexStore, WalletStatus,
};
