//! Enrich Electrum scripthash history rows with amounts and timestamps from raw txs.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd::p2pkh_script_to_address;
use vericonomy_tx::{decode_verium_tx, VeriumMutableTx};

use crate::history_rows::rows_from_decoded_tx;
use crate::types::WalletTx;

#[async_trait]
pub trait HistoryTxFetcher {
    async fn fetch_raw_tx_hex(&self, txid: &str) -> Result<String>;
}

fn script_matches_wallet(script: &[u8], wallet_scripts: &HashSet<Vec<u8>>) -> bool {
    wallet_scripts.iter().any(|s| s.as_slice() == script)
}

fn decode_tx_hex(raw_hex: &str) -> Result<VeriumMutableTx> {
    let hex = crate::tx_hex::normalize_transaction_hex(raw_hex, "tx hex")?;
    let bytes = hex::decode(&hex)
        .map_err(|e| WalletError::other(format!("tx hex decode: {e}")))?;
    decode_verium_tx(&bytes).map_err(|e| WalletError::other(format!("tx decode: {e}")))
}

async fn ensure_decoded(
    txid: &str,
    cache: &mut HashMap<String, VeriumMutableTx>,
    fetcher: &impl HistoryTxFetcher,
) -> Result<()> {
    if cache.contains_key(txid) {
        return Ok(());
    }
    let raw = fetcher.fetch_raw_tx_hex(txid).await?;
    cache.insert(txid.to_string(), decode_tx_hex(&raw)?);
    Ok(())
}

/// Copy enriched fields from a prior cache generation so sync refreshes do not wipe them.
pub fn merge_preserved_enrichment(new_rows: &mut [WalletTx], existing: &[WalletTx]) {
    let prior: HashMap<String, &WalletTx> = existing
        .iter()
        .map(|t| (crate::history_rows::wallet_tx_row_key(t), t))
        .collect();
    for row in new_rows {
        if let Some(old) = prior.get(&crate::history_rows::wallet_tx_row_key(row)) {
            if old.time.is_some() {
                row.time = old.time;
                row.amount = old.amount;
                row.address.clone_from(&old.address);
                row.category.clone_from(&old.category);
                if row.fee_sats.is_none() {
                    row.fee_sats = old.fee_sats;
                }
            }
        }
    }
}

/// Expand scripthash history stubs into Core-style send/receive rows.
pub async fn expand_wallet_history_rows(
    coin: CoinId,
    script_hexes: &[String],
    txs: &[WalletTx],
    fetcher: &impl HistoryTxFetcher,
    max_rows: Option<usize>,
    tip: Option<u32>,
) -> Result<Vec<WalletTx>> {
    let wallet_scripts: HashSet<Vec<u8>> = script_hexes
        .iter()
        .filter_map(|h| hex::decode(h.trim()).ok())
        .collect();
    if wallet_scripts.is_empty() || txs.is_empty() {
        return Ok(Vec::new());
    }

    let wallet_addresses: HashSet<String> = script_hexes
        .iter()
        .filter_map(|h| {
            hex::decode(h.trim())
                .ok()
                .and_then(|script| p2pkh_script_to_address(coin, &script))
        })
        .collect();

    let mut decoded_cache: HashMap<String, VeriumMutableTx> = HashMap::new();
    let mut expanded = 0usize;
    let mut rows = Vec::new();

    for row in txs {
        if let Some(max) = max_rows {
            if expanded >= max {
                break;
            }
        }

        let row_txid = row.txid.clone();
        if let Err(e) = ensure_decoded(&row_txid, &mut decoded_cache, fetcher).await {
            tracing::debug!("history expand skip {}: {e}", row_txid);
            continue;
        }

        let prev_txids: Vec<String> = decoded_cache
            .get(&row_txid)
            .map(|tx| {
                tx.inputs
                    .iter()
                    .map(|input| input.previous_output.txid.to_string())
                    .collect()
            })
            .unwrap_or_default();

        for prev_txid in prev_txids {
            if let Err(e) = ensure_decoded(&prev_txid, &mut decoded_cache, fetcher).await {
                tracing::debug!("history expand prev {} for {}: {e}", prev_txid, row_txid);
            }
        }

        let decoded = decoded_cache.get(&row_txid).unwrap();
        let input_refs: Vec<(String, u32)> = decoded
            .inputs
            .iter()
            .map(|input| {
                (
                    input.previous_output.txid.to_string(),
                    input.previous_output.vout,
                )
            })
            .collect();

        let mut prev_output_is_ours: HashSet<(String, u32)> = HashSet::new();
        for (txid, vout) in &input_refs {
            let prev_tx = match decoded_cache.get(txid) {
                Some(t) => t,
                None => continue,
            };
            let vout_idx = *vout as usize;
            if vout_idx >= prev_tx.outputs.len() {
                continue;
            }
            let prev_script = prev_tx.outputs[vout_idx].script_pubkey.as_bytes();
            if script_matches_wallet(prev_script, &wallet_scripts) {
                prev_output_is_ours.insert((txid.clone(), *vout));
            }
        }

        rows.extend(rows_from_decoded_tx(
            coin,
            &row_txid,
            decoded,
            row.height,
            tip,
            &wallet_scripts,
            &wallet_addresses,
            &prev_output_is_ours,
        ));
        expanded += 1;
    }

    Ok(rows)
}

/// Legacy single-row enrich path (kept for callers that still mutate one row per txid).
pub async fn enrich_wallet_history(
    coin: CoinId,
    script_hexes: &[String],
    txs: &mut [WalletTx],
    fetcher: &impl HistoryTxFetcher,
    max_rows: Option<usize>,
) -> Result<()> {
    let tip = txs.iter().filter_map(|t| t.blockheight).max();
    let expanded =
        expand_wallet_history_rows(coin, script_hexes, txs, fetcher, max_rows, tip).await?;
    let by_txid: HashMap<String, WalletTx> = expanded
        .into_iter()
        .map(|row| (row.txid.clone(), row))
        .collect();
    for row in txs.iter_mut() {
        if let Some(first) = by_txid.get(&row.txid) {
            row.time = first.time;
            row.amount = first.amount;
            row.address.clone_from(&first.address);
            row.category.clone_from(&first.category);
        }
    }
    Ok(())
}

/// History cache written before enrichment stored zero amounts and no timestamps.
pub fn history_rows_need_enrichment(txs: &[WalletTx]) -> bool {
    txs.iter().any(|t| !t.txid.is_empty() && t.time.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unenriched_rows() {
        let rows = vec![WalletTx {
            txid: "abc".into(),
            height: 1,
            fee_sats: None,
            category: "receive".into(),
            amount: 0.0,
            address: None,
            confirmations: 1,
            time: None,
            blockhash: None,
            blockheight: Some(1),
        }];
        assert!(history_rows_need_enrichment(&rows));
    }
}
