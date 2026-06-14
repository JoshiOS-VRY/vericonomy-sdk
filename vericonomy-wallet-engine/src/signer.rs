//! Local transaction signing for light wallet (secp256k1, never sent to server).

use bitcoin::absolute::LockTime;
use bitcoin::transaction::Version;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness};
use vericonomy_chain::types::Utxo;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd::{
    address_to_script_pubkey, derive_keypair_on_chain, pubkey_to_p2pkh_address, uses_core_hd_paths,
    GAP_SCAN_MAX_INDEX, HdChain,
};
use vericonomy_tx::{
    build_signed_tx_hex, current_n_time, display_txid_from_raw, parse_display_txid,
    serialize_verium_tx, SignInput, VeriumMutableTx,
};

use crate::utxo_selector::DUST_CHANGE_SATS;

pub struct SignedTx {
    pub hex: String,
    pub txid: String,
}

fn assemble_transaction(
    coin: CoinId,
    mnemonic: &str,
    bip39_passphrase: Option<&str>,
    utxos: &[Utxo],
    outputs: &[(String, i64)],
    change_address: &str,
    fee_sats: i64,
) -> Result<(VeriumMutableTx, Vec<[u8; 32]>)> {
    let mut inputs = Vec::new();
    let mut input_keys = Vec::new();
    let mut input_sum = 0i64;
    for utxo in utxos {
        if utxo.script_hex.is_empty() {
            return Err(WalletError::other("utxo missing script for signing"));
        }
        let txid = parse_display_txid(&utxo.txid)?;
        inputs.push(TxIn {
            previous_output: OutPoint {
                txid,
                vout: utxo.vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        });
        input_sum += utxo.value_sats;
        let script_bytes = hex::decode(utxo.script_hex.trim())
            .map_err(|e| WalletError::other(format!("utxo script hex: {e}")))?;
        let (secret, _) =
            find_signing_key(coin, mnemonic, bip39_passphrase, &utxo.address, &script_bytes)?;
        input_keys.push(secret);
    }
    let output_sum: i64 = outputs.iter().map(|(_, v)| *v).sum();
    let change_sats = input_sum - output_sum - fee_sats;
    if change_sats < 0 {
        return Err(WalletError::insufficient_funds(
            "insufficient funds for outputs + fee",
        ));
    }
    let mut tx_outputs = Vec::new();
    for (addr, value) in outputs {
        let script = address_to_script_pubkey(coin, addr)?;
        tx_outputs.push(TxOut {
            value: Amount::from_sat((*value).max(0) as u64),
            script_pubkey: ScriptBuf::from_bytes(script),
        });
    }
    if change_sats > DUST_CHANGE_SATS {
        let script = address_to_script_pubkey(coin, change_address)?;
        tx_outputs.push(TxOut {
            value: Amount::from_sat(change_sats as u64),
            script_pubkey: ScriptBuf::from_bytes(script),
        });
    }
    Ok((
        VeriumMutableTx {
            version: Version::ONE.0,
            n_time: current_n_time(),
            inputs,
            outputs: tx_outputs,
            lock_time: LockTime::ZERO.to_consensus_u32(),
        },
        input_keys,
    ))
}

pub fn build_unsigned_hex(
    coin: CoinId,
    mnemonic: &str,
    bip39_passphrase: Option<&str>,
    utxos: &[Utxo],
    outputs: &[(String, i64)],
    change_address: &str,
    fee_sats: i64,
) -> Result<String> {
    let (tx, _) = assemble_transaction(
        coin,
        mnemonic,
        bip39_passphrase,
        utxos,
        outputs,
        change_address,
        fee_sats,
    )?;
    Ok(hex::encode(serialize_verium_tx(&tx)?))
}

pub fn sign_transaction(
    coin: CoinId,
    mnemonic: &str,
    bip39_passphrase: Option<&str>,
    utxos: &[Utxo],
    outputs: &[(String, i64)],
    change_address: &str,
    fee_sats: i64,
) -> Result<SignedTx> {
    let (mut tx, input_keys) = assemble_transaction(
        coin,
        mnemonic,
        bip39_passphrase,
        utxos,
        outputs,
        change_address,
        fee_sats,
    )?;
    let sign_inputs: Vec<SignInput> = utxos
        .iter()
        .map(|u| SignInput {
            script_hex: u.script_hex.clone(),
        })
        .collect();
    let hex = build_signed_tx_hex(&mut tx, &sign_inputs, &input_keys)?;
    let raw = hex::decode(&hex).map_err(|e| WalletError::other(format!("signed tx hex: {e}")))?;
    Ok(SignedTx {
        hex,
        txid: display_txid_from_raw(&raw),
    })
}

fn find_signing_key(
    coin: CoinId,
    mnemonic: &str,
    bip39_passphrase: Option<&str>,
    address: &str,
    expected_script_pubkey: &[u8],
) -> Result<([u8; 32], Vec<u8>)> {
    if address.is_empty() {
        return Err(WalletError::other("utxo missing address for signing"));
    }
    if uses_core_hd_paths(coin, mnemonic) {
        for chain in [HdChain::External, HdChain::Internal] {
            for index in 0..GAP_SCAN_MAX_INDEX {
                let (secret, pubkey) =
                    derive_keypair_on_chain(coin, mnemonic, bip39_passphrase, chain, index)?;
                let addr = pubkey_to_p2pkh_address(coin, &pubkey)?;
                if addr == address {
                    let script = address_to_script_pubkey(coin, &addr)?;
                    if script == expected_script_pubkey {
                        return Ok((secret, pubkey));
                    }
                }
            }
        }
    } else {
        for index in 0..GAP_SCAN_MAX_INDEX {
            let (secret, pubkey) =
                derive_keypair_on_chain(coin, mnemonic, bip39_passphrase, HdChain::External, index)?;
            let addr = pubkey_to_p2pkh_address(coin, &pubkey)?;
            if addr == address {
                let script = address_to_script_pubkey(coin, &addr)?;
                if script == expected_script_pubkey {
                    return Ok((secret, pubkey));
                }
            }
        }
    }
    Err(WalletError::other(format!(
        "no signing key found for address {address} matching output script"
    )))
}
