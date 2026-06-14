//! Pluggable chain backends: local full node RPC or Electrum light client.

pub mod electrum;
pub mod full_node;
pub mod history_rows;
pub mod tx_hex;
pub mod types;

use async_trait::async_trait;

use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;
use vericonomy_hd::address_to_script_pubkey;

pub use types::*;

/// Remote read + broadcast surface shared by full-node and light wallets.
#[async_trait]
pub trait ChainBackend: Send + Sync {
    fn backend_kind(&self) -> BackendKind;
    fn connection_status(&self) -> ConnectionStatus;

    async fn get_tip(&self) -> Result<ChainTip>;
    async fn network_info(&self) -> Result<NetworkInfo>;

    async fn get_balance_for_scripts(&self, script_hexes: &[String]) -> Result<WalletBalance>;
    /// Per-script balances in the same order as `script_hexes` (for parallel gap scan).
    async fn get_balances_per_script(&self, script_hexes: &[String]) -> Result<Vec<WalletBalance>>;
    async fn list_utxos_for_scripts(&self, script_hexes: &[String]) -> Result<Vec<Utxo>>;
    async fn get_history_for_scripts(
        &self,
        script_hexes: &[String],
        limit: usize,
    ) -> Result<Vec<WalletTx>>;

    /// Fill in amount/time/address on history rows (Electrum only). No-op on full node.
    async fn enrich_tx_history_batch(
        &self,
        _coin: CoinId,
        _script_hexes: &[String],
        _txs: &mut [WalletTx],
        _max_rows: usize,
    ) -> Result<()> {
        Ok(())
    }

    async fn get_raw_tx_hex(&self, txid: &str) -> Result<String>;
    async fn estimate_fee(&self, target_blocks: u32) -> Result<FeeRate>;
    async fn broadcast_tx(&self, raw_hex: &str) -> Result<String>;

    /// Light-mode only: current Electrum server status.
    async fn light_server_status(&self) -> Option<LightServerStatus> {
        None
    }

    /// Cap Electrum RPC rate during initial HD indexing. No-op on full node.
    async fn set_initial_indexing_limits(
        &self,
        _max_scripthash_rpcs: u32,
        _max_scripts_per_batch: u32,
    ) {
    }

    /// True when the indexing RPC budget for this sync slice is exhausted.
    fn indexing_budget_exhausted(&self) -> bool {
        false
    }

    /// Clear indexing limits after a sync slice completes.
    async fn clear_initial_indexing_limits(&self) {}
}

pub fn script_hex_from_address(coin: CoinId, address: &str) -> Result<String> {
    let script = address_to_script_pubkey(coin, address)?;
    Ok(hex::encode(script))
}
