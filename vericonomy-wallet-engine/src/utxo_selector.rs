//! Simple UTXO selection for light wallet sends.

use vericonomy_chain::types::Utxo;
use vericonomy_errors::{Result, WalletError};

/// Verium VIP1+ network minimum fee rate: 100000 satoshis per 1000 bytes (0.001 VRM/kB).
/// See `VIP1_MIN_TX_FEE` in `verium/src/validation.h`.
pub const VIP1_MIN_TX_FEE_PER_K: i64 = 100_000;

/// Default wallet fee rate matching the legacy Qt UI and relay policy.
pub const DEFAULT_TX_FEE_COINS_PER_KB: f64 = 0.001;

/// P2PKH dust threshold at Verium's default dust relay rate (3000 sat/kB).
pub const DUST_CHANGE_SATS: i64 = 546;

pub fn select_utxos(utxos: &[Utxo], target_sats: i64, fee_sats: i64) -> Result<Vec<Utxo>> {
    let needed = target_sats + fee_sats;
    if needed <= 0 {
        return Err(WalletError::insufficient_funds("invalid send amount"));
    }

    let mut sufficient: Vec<&Utxo> = utxos
        .iter()
        .filter(|u| u.value_sats >= needed)
        .collect();
    sufficient.sort_by_key(|u| u.value_sats);
    if let Some(best) = sufficient.first() {
        return Ok(vec![(*best).clone()]);
    }

    let mut sorted: Vec<&Utxo> = utxos.iter().collect();
    sorted.sort_by_key(|u| u.value_sats);

    let mut selected = Vec::new();
    let mut sum = 0i64;
    for u in sorted {
        selected.push(u.clone());
        sum += u.value_sats;
        if sum >= needed {
            return Ok(selected);
        }
    }
    Err(WalletError::insufficient_funds("insufficient funds"))
}

/// Verium wire-format P2PKH size (includes the extra `nTime` field after version).
pub fn estimate_tx_size(input_count: usize, output_count: usize) -> usize {
    14 + input_count * 148 + output_count * 34
}

/// Verium `CFeeRate::GetFee(nBytes, addStartFee)` from `policy/feerate.cpp`.
pub fn verium_fee_for_size(satoshis_per_k: i64, n_bytes: usize, add_start_fee: bool) -> i64 {
    let n_size = n_bytes as i64;
    let mut n_fee = satoshis_per_k * (n_size / 1000);
    if add_start_fee {
        n_fee += satoshis_per_k;
    }
    if n_fee == 0 && n_size != 0 && satoshis_per_k > 0 {
        n_fee = 1;
    }
    n_fee
}

pub fn fee_for_rate(rate_coins_per_kb: f64, input_count: usize, output_count: usize) -> i64 {
    let bytes = estimate_tx_size(input_count, output_count);
    let user_rate_sats_per_k = coins_per_kb_to_sats_per_k(rate_coins_per_kb);
    let user_fee = verium_fee_for_size(user_rate_sats_per_k, bytes, true);
    let network_min = verium_fee_for_size(VIP1_MIN_TX_FEE_PER_K, bytes, true);
    user_fee.max(network_min)
}

pub fn coins_per_kb_to_sats_per_k(rate_coins_per_kb: f64) -> i64 {
    (rate_coins_per_kb * 100_000_000.0).round() as i64
}

fn change_will_be_included(input_sum: i64, amount_sats: i64, fee_sats: i64) -> bool {
    input_sum - amount_sats - fee_sats > DUST_CHANGE_SATS
}

pub fn plan_send_utxos(
    utxos: &[Utxo],
    amount_sats: i64,
    rate_coins_per_kb: f64,
    recipient_outputs: usize,
) -> Result<(Vec<Utxo>, i64)> {
    let recipient_outputs = recipient_outputs.max(1);
    let mut fee_sats = fee_for_rate(rate_coins_per_kb, 1, recipient_outputs + 1);
    let mut selected = select_utxos(utxos, amount_sats, fee_sats)?;

    for _ in 0..8 {
        let selected_sum: i64 = selected.iter().map(|u| u.value_sats).sum();
        let output_count = if change_will_be_included(selected_sum, amount_sats, fee_sats) {
            recipient_outputs + 1
        } else {
            recipient_outputs
        };
        let new_fee = fee_for_rate(rate_coins_per_kb, selected.len(), output_count);
        let needed = amount_sats + new_fee;
        if new_fee != fee_sats || selected_sum < needed {
            fee_sats = new_fee;
            selected = select_utxos(utxos, amount_sats, fee_sats)?;
            continue;
        }
        return Ok((selected, fee_sats));
    }
    Err(WalletError::other("could not converge send fee plan"))
}

pub fn replan_fee_for_selected(
    selected: &[Utxo],
    amount_sats: i64,
    rate_coins_per_kb: f64,
    recipient_outputs: usize,
) -> Result<i64> {
    if selected.is_empty() {
        return Err(WalletError::other("no inputs selected for send"));
    }
    let recipient_outputs = recipient_outputs.max(1);
    let input_sum: i64 = selected.iter().map(|u| u.value_sats).sum();
    let mut fee_sats = fee_for_rate(rate_coins_per_kb, selected.len(), recipient_outputs + 1);
    for _ in 0..8 {
        let output_count = if change_will_be_included(input_sum, amount_sats, fee_sats) {
            recipient_outputs + 1
        } else {
            recipient_outputs
        };
        let new_fee = fee_for_rate(rate_coins_per_kb, selected.len(), output_count);
        if new_fee == fee_sats {
            if input_sum < amount_sats + new_fee {
                return Err(WalletError::insufficient_funds(format!(
                    "insufficient funds: need {} sats (amount + fee) but inputs total {} sats",
                    amount_sats + new_fee,
                    input_sum
                )));
            }
            return Ok(new_fee);
        }
        fee_sats = new_fee;
    }
    Err(WalletError::other(
        "could not converge send fee after refreshing inputs",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vericonomy_chain::types::Utxo;

    #[test]
    fn verium_start_fee_is_at_least_one_per_k_rate() {
        let fee = verium_fee_for_size(VIP1_MIN_TX_FEE_PER_K, 230, true);
        assert_eq!(fee, VIP1_MIN_TX_FEE_PER_K);
    }

    #[test]
    fn low_user_rate_still_meets_network_minimum() {
        let fee = fee_for_rate(0.0001, 1, 2);
        assert_eq!(fee, VIP1_MIN_TX_FEE_PER_K);
    }

    #[test]
    fn default_rate_matches_network_minimum_for_typical_send() {
        let fee = fee_for_rate(DEFAULT_TX_FEE_COINS_PER_KB, 1, 2);
        assert_eq!(fee, VIP1_MIN_TX_FEE_PER_K);
    }

    #[test]
    fn select_utxos_prefers_smallest_sufficient_input() {
        let small = Utxo {
            txid: "small".repeat(64),
            vout: 0,
            value_sats: 7_000_000_000,
            height: 1,
            address: String::new(),
            script_hex: String::new(),
            confirmations: 10,
        };
        let large = Utxo {
            txid: "large".repeat(64),
            vout: 0,
            value_sats: 28_000_000_000,
            height: 1,
            address: String::new(),
            script_hex: String::new(),
            confirmations: 10,
        };
        let amount_sats = 100_000_000;
        let fee_sats = VIP1_MIN_TX_FEE_PER_K;
        let selected = select_utxos(&[large, small], amount_sats, fee_sats).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].value_sats, 7_000_000_000);
    }

    #[test]
    fn select_utxos_accumulates_smallest_when_no_single_input_covers() {
        let a = Utxo {
            txid: "a".repeat(64),
            vout: 0,
            value_sats: 40_000_000,
            height: 1,
            address: String::new(),
            script_hex: String::new(),
            confirmations: 1,
        };
        let b = Utxo {
            txid: "b".repeat(64),
            vout: 0,
            value_sats: 50_000_000,
            height: 1,
            address: String::new(),
            script_hex: String::new(),
            confirmations: 1,
        };
        let amount_sats = 80_000_000;
        let fee_sats = VIP1_MIN_TX_FEE_PER_K;
        let selected = select_utxos(&[b, a], amount_sats, fee_sats).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].value_sats, 40_000_000);
        assert_eq!(selected[1].value_sats, 50_000_000);
    }

    #[test]
    fn plan_send_converges_for_high_custom_rate() {
        let utxos = vec![Utxo {
            txid: "a".repeat(64),
            vout: 0,
            value_sats: 100_000_000,
            height: 1,
            address: String::new(),
            script_hex: String::new(),
            confirmations: 1,
        }];
        let (selected, fee) = plan_send_utxos(&utxos, 50_000_000, 0.5, 1).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(fee, 50_000_000);
    }
}
