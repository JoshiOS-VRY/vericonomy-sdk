//! Transaction history merge helpers.

use vericonomy_chain::types::WalletTx;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;

pub use vericonomy_chain::history_rows::wallet_tx_row_key;

pub fn tx_history_confirmed(tx: &WalletTx) -> bool {
    tx.height > 0 || tx.blockheight.is_some()
}

pub fn merge_tx_history(base: Vec<WalletTx>, pending: &[WalletTx]) -> Vec<WalletTx> {
    use std::collections::HashMap;

    let mut map: HashMap<String, WalletTx> = base
        .into_iter()
        .map(|tx| (wallet_tx_row_key(&tx), tx))
        .collect();
    for tx in pending {
        let key = wallet_tx_row_key(tx);
        if let Some(existing) = map.get(&key) {
            if tx_history_confirmed(existing) && !tx_history_confirmed(tx) {
                continue;
            }
        }
        map.insert(key, tx.clone());
    }
    let mut rows: Vec<_> = map.into_values().collect();
    rows.sort_by(|a, b| {
        let ta = a.time.unwrap_or(0);
        let tb = b.time.unwrap_or(0);
        tb.cmp(&ta).then(b.height.cmp(&a.height))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use vericonomy_chain::types::WalletTx;

    fn tx(txid: &str, height: i32) -> WalletTx {
        WalletTx {
            txid: txid.to_string(),
            height,
            fee_sats: None,
            category: "receive".to_string(),
            amount: 1.0,
            address: None,
            confirmations: height.max(0),
            time: Some(1),
            blockhash: None,
            blockheight: None,
        }
    }

    #[test]
    fn merge_prefers_confirmed_over_pending_duplicate() {
        let base = vec![tx("abc", 100)];
        let pending = vec![tx("abc", 0)];
        let merged = merge_tx_history(base, &pending);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].height, 100);
    }

    #[test]
    fn merge_includes_new_pending_rows() {
        let base = vec![tx("abc", 100)];
        let pending = vec![tx("def", 0)];
        let merged = merge_tx_history(base, &pending);
        assert_eq!(merged.len(), 2);
    }
}

/// Optional explorer-backed history (HTTP impl provided by platform shell).
#[async_trait::async_trait]
pub trait HistorySource: Send + Sync {
    async fn fetch_wallet_history(
        &self,
        coin: CoinId,
        addresses: &[String],
        limit: usize,
        tip: Option<u32>,
    ) -> Result<Vec<WalletTx>>;
}

pub struct NoopHistorySource;

#[async_trait::async_trait]
impl HistorySource for NoopHistorySource {
    async fn fetch_wallet_history(
        &self,
        _coin: CoinId,
        _addresses: &[String],
        _limit: usize,
        _tip: Option<u32>,
    ) -> Result<Vec<WalletTx>> {
        Ok(Vec::new())
    }
}
