//! Cache-aware balance and post-send optimistic updates.

use std::collections::HashSet;

use vericonomy_chain::types::{Utxo, WalletBalance, WalletTx};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;
use vericonomy_hd::address_to_script_pubkey;
use vericonomy_storage::{TxCache, UtxoCache, WalletIndexStore};
use vericonomy_tx::decode_verium_tx;
use vericonomy_wallet_engine::utxo_selector::DUST_CHANGE_SATS;

use crate::history::merge_tx_history;
use vericonomy_chain::history_rows;

pub const TX_HISTORY_CACHE_LIMIT: usize = usize::MAX;
const OPTIMISTIC_UTXO_META: &str = "optimistic_utxo_keys";
const LAST_ELECTRUM_UTXO_META: &str = "last_electrum_utxo_keys";

pub async fn balance_from_utxo_cache<U: UtxoCache>(
    cache: &U,
    coin: CoinId,
) -> Result<WalletBalance> {
    let mut confirmed = 0i64;
    let mut unconfirmed = 0i64;
    let utxos = cache.list_utxos(coin).await.unwrap_or_default();
    let optimistic = parse_utxo_key_set(cache.get_meta(coin, OPTIMISTIC_UTXO_META).await?);
    let electrum_keys = parse_utxo_key_set(cache.get_meta(coin, LAST_ELECTRUM_UTXO_META).await?);
    let electrum_snapshot_ready = cache
        .get_meta(coin, LAST_ELECTRUM_UTXO_META)
        .await?
        .is_some();

    for utxo in utxos {
        if utxo.height > 0 {
            confirmed += utxo.value_sats;
            continue;
        }
        let key = (utxo.txid.clone(), utxo.vout);
        let trusted =
            optimistic.contains(&key) || (electrum_snapshot_ready && electrum_keys.contains(&key));
        if trusted {
            unconfirmed += utxo.value_sats;
        } else if electrum_snapshot_ready {
            let _ = cache.remove_utxo(coin, &utxo.txid, utxo.vout).await;
        } else {
            unconfirmed += utxo.value_sats;
        }
    }

    Ok(WalletBalance {
        confirmed_sats: confirmed,
        unconfirmed_sats: unconfirmed,
        immature_sats: 0,
    })
}

pub async fn apply_local_send_cache_update<U, T, I>(
    cache: &U,
    tx_cache: &T,
    index: &I,
    coin: CoinId,
    spent: &[Utxo],
    broadcast_txid: &str,
    signed_hex: &str,
    change_sats: i64,
    change_address: &str,
) -> Result<()>
where
    U: UtxoCache,
    T: TxCache,
    I: WalletIndexStore,
{
    let raw = hex::decode(signed_hex.trim())
        .map_err(|e| vericonomy_errors::WalletError::other(format!("post-send tx hex: {e}")))?;
    let tx = decode_verium_tx(&raw)?;
    let output_count = tx.outputs.len();

    for utxo in spent {
        cache.remove_utxo(coin, &utxo.txid, utxo.vout).await?;
    }
    if change_sats > DUST_CHANGE_SATS && output_count > 0 {
        let script = address_to_script_pubkey(coin, change_address)?;
        let change_vout = (output_count - 1) as u32;
        cache
            .upsert_utxo(
                coin,
                &Utxo {
                    txid: broadcast_txid.to_string(),
                    vout: change_vout,
                    value_sats: change_sats,
                    script_hex: hex::encode(script),
                    height: 0,
                    address: change_address.to_string(),
                    confirmations: 0,
                },
            )
            .await?;
        register_optimistic_utxo(cache, coin, broadcast_txid, change_vout).await?;
    }

    let wallet_addresses: HashSet<String> = spent
        .iter()
        .map(|u| u.address.clone())
        .filter(|a| !a.is_empty())
        .chain(std::iter::once(change_address.to_string()))
        .collect();

    let mut wallet_scripts: HashSet<Vec<u8>> = HashSet::new();
    for utxo in spent {
        if let Ok(bytes) = hex::decode(utxo.script_hex.trim()) {
            wallet_scripts.insert(bytes);
        }
    }
    if change_sats > DUST_CHANGE_SATS {
        if let Ok(script) = address_to_script_pubkey(coin, change_address) {
            wallet_scripts.insert(script);
        }
    }

    let mut prev_output_is_ours = HashSet::new();
    for input in &tx.inputs {
        prev_output_is_ours.insert((
            input.previous_output.txid.to_string(),
            input.previous_output.vout,
        ));
    }

    let history_rows = history_rows::rows_from_decoded_tx(
        coin,
        broadcast_txid,
        &tx,
        0,
        None,
        &wallet_scripts,
        &wallet_addresses,
        &prev_output_is_ours,
    );
    append_local_tx_history_rows(cache, tx_cache, coin, &history_rows).await?;
    for utxo in spent {
        if !utxo.script_hex.is_empty() {
            let _ = index.register_funded_script_hex(coin, &utxo.script_hex).await;
        }
    }
    if change_sats > DUST_CHANGE_SATS {
        if let Ok(script) = address_to_script_pubkey(coin, change_address) {
            let _ = index
                .register_funded_script_hex(coin, &hex::encode(script))
                .await;
        }
    }
    Ok(())
}

pub async fn store_last_electrum_utxo_keys<U: UtxoCache>(
    cache: &U,
    coin: CoinId,
    utxos: &[Utxo],
) -> Result<()> {
    let keys: Vec<String> = utxos
        .iter()
        .map(|u| format!("{}:{}", u.txid, u.vout))
        .collect();
    cache
        .set_meta(coin, LAST_ELECTRUM_UTXO_META, &keys.join(","))
        .await
}

async fn register_optimistic_utxo<U: UtxoCache>(
    cache: &U,
    coin: CoinId,
    txid: &str,
    vout: u32,
) -> Result<()> {
    let mut keys = parse_utxo_key_set(cache.get_meta(coin, OPTIMISTIC_UTXO_META).await?);
    keys.insert((txid.to_string(), vout));
    let serialized = keys
        .iter()
        .map(|(t, v)| format!("{t}:{v}"))
        .collect::<Vec<_>>()
        .join(",");
    cache.set_meta(coin, OPTIMISTIC_UTXO_META, &serialized).await
}

async fn append_local_tx_history_rows<U: UtxoCache, T: TxCache>(
    cache: &U,
    tx_cache: &T,
    coin: CoinId,
    rows: &[WalletTx],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let existing = tx_cache
        .load_history(coin, TX_HISTORY_CACHE_LIMIT)
        .await
        .unwrap_or_default();
    let merged = merge_tx_history(existing, rows);
    tx_cache.replace_history(coin, &merged).await?;
    let _ = cache;
    Ok(())
}

fn parse_utxo_key_set(raw: Option<String>) -> HashSet<(String, u32)> {
    let mut out = HashSet::new();
    let Some(s) = raw else {
        return out;
    };
    for part in s.split(',') {
        if let Some((txid, vout)) = part.split_once(':') {
            if let Ok(v) = vout.parse::<u32>() {
                out.insert((txid.to_string(), v));
            }
        }
    }
    out
}

pub fn wallet_tx_row_key_from_tx(tx: &WalletTx) -> String {
    history_rows::wallet_tx_row_key(tx)
}
