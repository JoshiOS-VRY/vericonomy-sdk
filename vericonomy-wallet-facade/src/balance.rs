//! Balance helpers derived from UTXO sets.

use vericonomy_chain::types::{Utxo, WalletBalance};

/// Only confirmed outputs are reliably spendable.
pub fn spendable_utxos(utxos: Vec<Utxo>) -> Vec<Utxo> {
    utxos.into_iter().filter(|u| u.height > 0).collect()
}

/// Aggregate confirmed/unconfirmed totals from a UTXO list.
pub fn balance_from_utxos(utxos: &[Utxo], tip_height: u32) -> WalletBalance {
    let mut confirmed_sats = 0i64;
    let mut unconfirmed_sats = 0i64;
    for utxo in utxos {
        if utxo.height == 0 {
            unconfirmed_sats += utxo.value_sats;
            continue;
        }
        let confs = tip_height.saturating_sub(utxo.height) + 1;
        if confs >= 1 {
            confirmed_sats += utxo.value_sats;
        } else {
            unconfirmed_sats += utxo.value_sats;
        }
    }
    WalletBalance {
        confirmed_sats,
        unconfirmed_sats,
        immature_sats: 0,
    }
}
