//! Build wallet history rows the same way Core `listtransactions` does.

use std::collections::HashSet;

use vericonomy_chain_params::CoinId;
use vericonomy_hd::{p2pkh_script_to_address, sats_to_coins};
use vericonomy_tx::VeriumMutableTx;

use crate::types::WalletTx;

pub fn wallet_tx_row_key(tx: &WalletTx) -> String {
    format!(
        "{}:{}:{}",
        tx.txid,
        tx.category,
        tx.address.as_deref().unwrap_or("")
    )
}

fn confirmations_for(tip: Option<u32>, block_height: Option<u64>) -> i32 {
    match (tip, block_height) {
        (Some(tip), Some(bh)) if tip >= bh as u32 => (tip - bh as u32 + 1) as i32,
        _ => 0,
    }
}

fn category_receive(height: i32) -> String {
    if height <= 0 {
        "unconfirmed".into()
    } else {
        "receive".into()
    }
}

struct OutputView {
    address: String,
    value_sats: i64,
}

fn make_row(
    txid: &str,
    height: i32,
    blockheight: Option<u32>,
    time: Option<u64>,
    blockhash: Option<String>,
    confirmations: i32,
    category: &str,
    amount_coins: f64,
    address: &str,
) -> WalletTx {
    WalletTx {
        txid: txid.to_string(),
        height,
        fee_sats: None,
        category: category.to_string(),
        amount: amount_coins,
        address: Some(address.to_string()),
        confirmations,
        time,
        blockhash,
        blockheight,
    }
}

fn change_output_address(wallet_outputs: &[OutputView]) -> Option<String> {
    wallet_outputs
        .iter()
        .max_by_key(|o| o.value_sats)
        .map(|o| o.address.clone())
}

fn rows_from_outputs(
    txid: &str,
    height: i32,
    blockheight: Option<u32>,
    time: Option<u64>,
    blockhash: Option<String>,
    confirmations: i32,
    is_spend: bool,
    wallet_outputs: Vec<OutputView>,
    external_outputs: Vec<OutputView>,
) -> Vec<WalletTx> {
    let mut rows = Vec::new();

    if !is_spend {
        for out in wallet_outputs {
            rows.push(make_row(
                txid,
                height,
                blockheight,
                time,
                blockhash.clone(),
                confirmations,
                &category_receive(height),
                sats_to_coins(out.value_sats),
                &out.address,
            ));
        }
        return rows;
    }

    let change_addr = change_output_address(&wallet_outputs);

    for out in wallet_outputs {
        rows.push(make_row(
            txid,
            height,
            blockheight,
            time,
            blockhash.clone(),
            confirmations,
            &category_receive(height),
            sats_to_coins(out.value_sats),
            &out.address,
        ));
        if change_addr.as_deref() != Some(out.address.as_str()) {
            rows.push(make_row(
                txid,
                height,
                blockheight,
                time,
                blockhash.clone(),
                confirmations,
                "send",
                -sats_to_coins(out.value_sats),
                &out.address,
            ));
        }
    }

    for out in external_outputs {
        rows.push(make_row(
            txid,
            height,
            blockheight,
            time,
            blockhash.clone(),
            confirmations,
            "send",
            -sats_to_coins(out.value_sats),
            &out.address,
        ));
    }

    rows
}

pub fn rows_from_decoded_tx(
    coin: CoinId,
    txid: &str,
    decoded: &VeriumMutableTx,
    height: i32,
    tip: Option<u32>,
    wallet_scripts: &HashSet<Vec<u8>>,
    wallet_addresses: &HashSet<String>,
    prev_output_is_ours: &HashSet<(String, u32)>,
) -> Vec<WalletTx> {
    let blockheight = if height > 0 {
        Some(height as u32)
    } else {
        None
    };
    let time = Some(decoded.n_time as u64);
    let confirmations = confirmations_for(tip, blockheight.map(|h| h as u64));

    let is_spend = decoded.inputs.iter().any(|input| {
        prev_output_is_ours.contains(&(
            input.previous_output.txid.to_string(),
            input.previous_output.vout,
        ))
    });

    let mut wallet_outputs = Vec::new();
    let mut external_outputs = Vec::new();

    for out in &decoded.outputs {
        let script = out.script_pubkey.as_bytes();
        let value_sats = out.value.to_sat() as i64;
        if value_sats <= 0 {
            continue;
        }
        let addr = p2pkh_script_to_address(coin, script);
        let view = OutputView {
            address: addr.unwrap_or_default(),
            value_sats,
        };
        if script_matches_wallet(script, wallet_scripts) {
            wallet_outputs.push(view);
        } else if !view.address.is_empty() {
            external_outputs.push(view);
        }
    }

    wallet_outputs.retain(|o| wallet_addresses.contains(o.address.as_str()));

    rows_from_outputs(
        txid,
        height,
        blockheight,
        time,
        None,
        confirmations,
        is_spend,
        wallet_outputs,
        external_outputs,
    )
}

fn script_matches_wallet(script: &[u8], wallet_scripts: &HashSet<Vec<u8>>) -> bool {
    wallet_scripts.iter().any(|s| s.as_slice() == script)
}
