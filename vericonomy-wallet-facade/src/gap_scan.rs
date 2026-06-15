//! HD gap scan and script precache (lifted from Tauri wallet/hd.rs).

use async_trait::async_trait;
use vericonomy_chain::ChainBackend;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;
use vericonomy_hd::{
    address_to_script_pubkey, derive_address_at, derive_address_on_chain, uses_core_hd_paths,
    GAP_SCAN_MAX_INDEX, HdChain,
};
use vericonomy_storage::IndexingProgress;

pub use vericonomy_storage::GAP_PRECACHE_MAX;

pub const GAP_LIMIT: u32 = 20;

pub struct ChainScanSlice {
    pub scripts: Vec<String>,
    pub next_index: u32,
    pub chain_complete: bool,
    pub budget_paused: bool,
}

#[async_trait]
pub trait GapScanHook: Send + Sync {
    async fn on_funded_batch(&self, funded: &[String], utxo_refresh: bool) -> Result<()>;
}

async fn discover_chain_script_hexes<B: ChainBackend>(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    chain: HdChain,
    gap_limit: u32,
    start_index: u32,
    backend: &B,
    batch_size: u32,
    funded_out: &mut Vec<String>,
    hook: Option<&dyn GapScanHook>,
    utxo_refresh_on_funded: bool,
) -> Result<ChainScanSlice> {
    let mut scripts = Vec::new();
    let mut consecutive_empty = 0u32;
    let mut index = start_index;
    while consecutive_empty < gap_limit && index <= GAP_SCAN_MAX_INDEX {
        let batch_end = (index + batch_size).min(GAP_SCAN_MAX_INDEX + 1);
        let mut batch_scripts = Vec::new();
        for idx in index..batch_end {
            let addr = derive_address_on_chain(coin, seed_secret, bip39_passphrase, chain, idx)?;
            let script = address_to_script_pubkey(coin, &addr)?;
            batch_scripts.push(hex::encode(&script));
        }
        let balances = match backend.get_balances_per_script(&batch_scripts).await {
            Ok(b) => b,
            Err(e) if e.is_indexing_budget_exhausted() => {
                return Ok(ChainScanSlice {
                    scripts,
                    next_index: index,
                    chain_complete: false,
                    budget_paused: true,
                });
            }
            Err(e) => return Err(e),
        };
        let mut batch_had_funded = false;
        for (script_hex, bal) in batch_scripts.into_iter().zip(balances) {
            if bal.total_sats() == 0 {
                consecutive_empty += 1;
            } else {
                consecutive_empty = 0;
                if !funded_out.iter().any(|s| s == &script_hex) {
                    funded_out.push(script_hex.clone());
                    batch_had_funded = true;
                }
            }
            scripts.push(script_hex);
            if consecutive_empty >= gap_limit {
                break;
            }
        }
        if batch_had_funded {
            if let Some(h) = hook {
                h.on_funded_batch(funded_out.as_slice(), utxo_refresh_on_funded)
                    .await?;
            }
        }
        index = batch_end;
        if consecutive_empty >= gap_limit {
            break;
        }
    }
    let chain_complete = consecutive_empty >= gap_limit || index > GAP_SCAN_MAX_INDEX;
    Ok(ChainScanSlice {
        scripts,
        next_index: index,
        chain_complete,
        budget_paused: false,
    })
}

pub async fn discover_script_hexes<B: ChainBackend>(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    gap_limit: u32,
    progress: &mut IndexingProgress,
    backend: &B,
    batch_size: u32,
    funded_out: &mut Vec<String>,
    hook: Option<&dyn GapScanHook>,
    utxo_refresh_on_funded: bool,
) -> Result<(Vec<String>, bool)> {
    let mut merged = Vec::new();

    if !progress.gap_external_done {
        let external = discover_chain_script_hexes(
            coin,
            seed_secret,
            bip39_passphrase,
            HdChain::External,
            gap_limit,
            progress.gap_external,
            backend,
            batch_size,
            funded_out,
            hook,
            utxo_refresh_on_funded,
        )
        .await?;
        merged.extend(external.scripts);
        progress.gap_external = external.next_index;
        if external.budget_paused {
            return Ok((merged, false));
        }
        if !external.chain_complete {
            return Ok((merged, false));
        }
        progress.gap_external_done = true;
    }

    if !uses_core_hd_paths(coin, seed_secret) {
        return Ok((merged, true));
    }

    if !progress.gap_internal_done {
        let internal = discover_chain_script_hexes(
            coin,
            seed_secret,
            bip39_passphrase,
            HdChain::Internal,
            gap_limit,
            progress.gap_internal,
            backend,
            batch_size,
            funded_out,
            hook,
            utxo_refresh_on_funded,
        )
        .await?;
        for script in internal.scripts {
            if !merged.contains(&script) {
                merged.push(script);
            }
        }
        progress.gap_internal = internal.next_index;
        if internal.budget_paused {
            return Ok((merged, false));
        }
        if !internal.chain_complete {
            return Ok((merged, false));
        }
        progress.gap_internal_done = true;
    }
    Ok((merged, true))
}

pub fn resolve_addresses_for_script_hexes(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    script_hexes: &[&str],
) -> Result<std::collections::HashMap<String, String>> {
    use std::collections::{HashMap, HashSet};

    let targets: HashSet<&str> = script_hexes.iter().copied().collect();
    let mut map = HashMap::new();
    if targets.is_empty() {
        return Ok(map);
    }

    if uses_core_hd_paths(coin, seed_secret) {
        for chain in [HdChain::External, HdChain::Internal] {
            for index in 0..GAP_SCAN_MAX_INDEX {
                let addr = derive_address_on_chain(coin, seed_secret, bip39_passphrase, chain, index)?;
                let script_hex = hex::encode(address_to_script_pubkey(coin, &addr)?);
                if targets.contains(script_hex.as_str()) {
                    map.insert(script_hex, addr);
                    if map.len() == targets.len() {
                        return Ok(map);
                    }
                }
            }
        }
    } else {
        for index in 0..GAP_SCAN_MAX_INDEX {
            let addr = derive_address_at(coin, seed_secret, bip39_passphrase, index)?;
            let script_hex = hex::encode(address_to_script_pubkey(coin, &addr)?);
            if targets.contains(script_hex.as_str()) {
                map.insert(script_hex, addr);
                if map.len() == targets.len() {
                    return Ok(map);
                }
            }
        }
    }
    Ok(map)
}

pub fn enrich_utxo_addresses(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    utxos: &mut [vericonomy_chain::types::Utxo],
) -> Result<()> {
    let scripts: Vec<&str> = utxos
        .iter()
        .filter(|u| u.address.is_empty() && !u.script_hex.is_empty())
        .map(|u| u.script_hex.as_str())
        .collect();
    let map = resolve_addresses_for_script_hexes(coin, seed_secret, bip39_passphrase, &scripts)?;
    for utxo in utxos {
        if utxo.address.is_empty() {
            if let Some(addr) = map.get(&utxo.script_hex) {
                utxo.address = addr.clone();
            }
        }
    }
    Ok(())
}

/// Map a funded script to its external receive address, if it belongs to that chain.
pub fn external_receive_index_for_script(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    script_hex: &str,
) -> Result<Option<(u32, String)>> {
    let target = script_hex.trim();
    if target.is_empty() {
        return Ok(None);
    }
    for idx in 0..=GAP_SCAN_MAX_INDEX {
        let addr = derive_address_at(coin, seed_secret, bip39_passphrase, idx)?;
        let script = hex::encode(address_to_script_pubkey(coin, &addr)?);
        if script.eq_ignore_ascii_case(target) {
            return Ok(Some((idx, addr)));
        }
    }
    Ok(None)
}

pub fn max_external_receive_index_for_scripts(
    coin: CoinId,
    seed_secret: &str,
    bip39_passphrase: Option<&str>,
    script_hexes: &[String],
) -> Result<u32> {
    let mut max_idx = 0u32;
    for script in script_hexes {
        if let Some((idx, _)) =
            external_receive_index_for_script(coin, seed_secret, bip39_passphrase, script)?
        {
            max_idx = max_idx.max(idx);
        }
    }
    Ok(max_idx)
}
