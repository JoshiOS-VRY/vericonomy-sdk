//! End-to-end light-wallet send pipeline.

use vericonomy_chain::types::Utxo;
use vericonomy_chain::ChainBackend;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;
use vericonomy_wallet_engine::{
    sign_transaction, validate_send_address, DEFAULT_TX_FEE_COINS_PER_KB,
};
use vericonomy_wallet_engine::utxo_selector::{plan_send_utxos, replan_fee_for_selected};

use crate::utxo_prep::prepare_utxos_for_signing;
use crate::verify::{verify_send_outputs, verify_signed_p2pkh_inputs};

#[derive(Debug, Clone)]
pub struct SendPaymentParams {
    pub recipient: String,
    pub amount_sats: i64,
    pub fee_rate_coins_per_kb: f64,
    pub utxos: Vec<vericonomy_chain::types::Utxo>,
    pub change_address: String,
    pub bip39_passphrase: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SendResult {
    pub txid: String,
    pub raw_hex: String,
    pub fee_sats: i64,
    pub change_sats: i64,
    pub spent_utxos: Vec<Utxo>,
}

/// Plan, sign, verify, and broadcast a single-recipient payment.
pub async fn send_payment<B: ChainBackend>(
    backend: &B,
    coin: CoinId,
    mnemonic: &str,
    params: SendPaymentParams,
) -> Result<SendResult> {
    validate_send_address(coin, &params.recipient)?;

    let rate = if params.fee_rate_coins_per_kb > 0.0 {
        params.fee_rate_coins_per_kb
    } else {
        DEFAULT_TX_FEE_COINS_PER_KB
    };

    let passphrase = params.bip39_passphrase.as_deref();
    let selected = params.utxos;
    if selected.is_empty() {
        return Err(vericonomy_errors::WalletError::insufficient_funds(
            "no spendable coins in wallet",
        ));
    }

    let (mut selected, _initial_fee) =
        plan_send_utxos(&selected, params.amount_sats, rate, 1)?;
    prepare_utxos_for_signing(backend, &mut selected).await?;
    let fee_sats = replan_fee_for_selected(&selected, params.amount_sats, rate, 1)?;

    let outputs = vec![(params.recipient.clone(), params.amount_sats)];
    let signed = sign_transaction(
        coin,
        mnemonic,
        passphrase,
        &selected,
        &outputs,
        &params.change_address,
        fee_sats,
    )?;

    verify_send_outputs(coin, &signed.hex, &outputs)?;
    verify_signed_p2pkh_inputs(&signed.hex, &selected)?;

    let txid = backend.broadcast_tx(&signed.hex).await?;
    let input_sum: i64 = selected.iter().map(|u| u.value_sats).sum();
    let change_sats = input_sum - params.amount_sats - fee_sats;

    Ok(SendResult {
        txid,
        raw_hex: signed.hex,
        fee_sats,
        change_sats,
        spent_utxos: selected,
    })
}
