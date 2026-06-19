//! Refresh UTXO metadata from parent transactions before signing.

use vericonomy_chain::ChainBackend;
use vericonomy_chain::types::Utxo;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd::address_to_script_pubkey;
use vericonomy_tx::{
    decode_verium_tx, display_txid_from_raw, parse_display_txid, reverse_display_txid_hex,
    wire_txid_from_raw,
};

use crate::verify::script_pays_to;

fn parent_txid_variants(txid: &str) -> Vec<String> {
    let trimmed = txid.trim();
    let mut variants = vec![trimmed.to_string()];
    if let Ok(alt) = reverse_display_txid_hex(trimmed) {
        if !variants.iter().any(|v| v.eq_ignore_ascii_case(&alt)) {
            variants.push(alt);
        }
    }
    variants
}

async fn fetch_parent_tx_bytes(backend: &dyn ChainBackend, txid: &str) -> Result<Vec<u8>> {
    let mut last_err: Option<WalletError> = None;
    for variant in parent_txid_variants(txid) {
        match backend.get_raw_tx_hex(&variant).await {
            Ok(hex) => {
                let bytes = hex::decode(&hex)
                    .map_err(|e| WalletError::other(format!("parent tx hex decode: {e}")))?;
                return Ok(bytes);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| WalletError::other("failed to fetch parent transaction")))
}

fn apply_parent_tx_to_utxo(utxo: &mut Utxo, bytes: &[u8]) -> Result<()> {
    let display_txid = display_txid_from_raw(bytes);
    let wire_txid = wire_txid_from_raw(bytes);
    let parsed = parse_display_txid(&display_txid)?;
    if parsed != wire_txid {
        return Err(WalletError::other(format!(
            "parent txid endian mismatch for {}",
            utxo.txid
        )));
    }

    let parent = decode_verium_tx(bytes)
        .map_err(|e| WalletError::other(format!("parent tx decode: {e}")))?;
    let out = parent
        .outputs
        .get(utxo.vout as usize)
        .ok_or_else(|| WalletError::other(format!("parent tx missing vout {}", utxo.vout)))?;
    let on_chain_script = hex::encode(out.script_pubkey.as_bytes());
    let on_chain_value = out.value.to_sat() as i64;

    if on_chain_value != utxo.value_sats {
        utxo.value_sats = on_chain_value;
    }

    if !script_pays_to(&on_chain_script, &utxo.script_hex) {
        return Err(WalletError::other(format!(
            "UTXO {}:{} on-chain script does not match wallet script",
            utxo.txid, utxo.vout
        )));
    }

    utxo.txid = display_txid;
    utxo.script_hex = on_chain_script;
    Ok(())
}

fn normalize_utxo_txid(utxo: &mut Utxo) -> Result<()> {
    if parse_display_txid(&utxo.txid).is_ok() {
        return Ok(());
    }
    if let Ok(alt) = reverse_display_txid_hex(&utxo.txid) {
        if parse_display_txid(&alt).is_ok() {
            utxo.txid = alt;
            return Ok(());
        }
    }
    Err(WalletError::other(format!(
        "invalid parent txid for UTXO {}:{}",
        utxo.txid, utxo.vout
    )))
}

/// Use wallet-local script/address metadata when Electrum cannot fetch a confirmed parent tx.
fn apply_utxo_from_wallet_cache(coin: CoinId, utxo: &mut Utxo) -> Result<()> {
    if utxo.address.is_empty() || utxo.script_hex.is_empty() {
        return Err(WalletError::other(format!(
            "cannot prepare UTXO {}:{} without address/script metadata",
            utxo.txid, utxo.vout
        )));
    }
    let derived = hex::encode(
        address_to_script_pubkey(coin, &utxo.address)
            .map_err(|e| WalletError::other(format!("derive spend script: {e}")))?,
    );
    if !script_pays_to(&derived, &utxo.script_hex) {
        return Err(WalletError::other(format!(
            "UTXO {}:{} script does not match derived address script",
            utxo.txid, utxo.vout
        )));
    }
    utxo.script_hex = derived;
    normalize_utxo_txid(utxo)
}

/// Fetch each parent tx from the chain backend, normalize txid/script/value, and verify the spend.
pub async fn prepare_utxos_for_signing(
    backend: &dyn ChainBackend,
    coin: CoinId,
    utxos: &mut [Utxo],
) -> Result<()> {
    if utxos.is_empty() {
        return Ok(());
    }
    for utxo in utxos.iter_mut() {
        match fetch_parent_tx_bytes(backend, &utxo.txid).await {
            Ok(bytes) => apply_parent_tx_to_utxo(utxo, &bytes)?,
            Err(e) if e.is_electrum_tx_lookup_failure() && utxo.height > 0 => {
                apply_utxo_from_wallet_cache(coin, utxo)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
