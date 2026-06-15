//! Wallet transaction history via the Vericonomy explorer index (same source as Tauri).

use std::collections::HashSet;

use reqwest::Client;
use serde::Deserialize;
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};

use crate::history::{wallet_tx_row_key, HistorySource};
use vericonomy_chain::types::WalletTx;

const PAGE_SIZE: u32 = 100;
const USER_AGENT: &str = "vericonomy-wallet-ios/1.0";

fn api_base(coin: CoinId) -> &'static str {
    match coin {
        CoinId::Verium => "https://explorer.vericonomy.com/v1/vrm",
        CoinId::Vericoin => "https://explorer.vericonomy.com/v1/vrc",
    }
}

fn http_client() -> Result<Client> {
    vericonomy_chain::ensure_tls_crypto_provider();
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| WalletError::other(format!("explorer http client: {e}")))
}

#[derive(Debug, Deserialize)]
struct AddressDetail {
    found: bool,
    #[serde(default)]
    paging: Option<Paging>,
    #[serde(default)]
    balance: Option<AddressBalance>,
    #[serde(default)]
    transactions: Vec<AddressTx>,
}

#[derive(Debug, Deserialize)]
struct Paging {
    #[serde(default)]
    total: Option<u64>,
    #[serde(default, rename = "hasMore")]
    has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AddressBalance {
    #[serde(default, rename = "txCount")]
    tx_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AddressTx {
    txid: String,
    #[serde(default, rename = "blockHeight")]
    block_height: Option<u64>,
    #[serde(default)]
    time: Option<u64>,
    #[serde(default, rename = "netDelta")]
    net_delta: Option<AmountField>,
}

#[derive(Debug, Deserialize)]
struct AmountField {
    amount: String,
}

fn parse_amount_coins(raw: &str) -> f64 {
    raw.trim().parse().unwrap_or(0.0)
}

fn confirmations_for(tip: Option<u32>, block_height: Option<u64>) -> i32 {
    match (tip, block_height) {
        (Some(tip), Some(bh)) if tip >= bh as u32 => (tip - bh as u32 + 1) as i32,
        _ => 0,
    }
}

fn row_from_address_tx(address: &str, tx: &AddressTx, tip: Option<u32>) -> Option<WalletTx> {
    let delta = parse_amount_coins(tx.net_delta.as_ref()?.amount.as_str());
    if delta.abs() < 0.000000_01 {
        return None;
    }
    let height = tx.block_height.map(|h| h as i32).unwrap_or(0);
    let category = if delta >= 0.0 {
        if height <= 0 {
            "unconfirmed".to_string()
        } else {
            "receive".to_string()
        }
    } else {
        "send".to_string()
    };
    Some(WalletTx {
        txid: tx.txid.clone(),
        height,
        fee_sats: None,
        category,
        amount: delta,
        address: Some(address.to_string()),
        confirmations: confirmations_for(tip, tx.block_height),
        time: tx.time,
        blockhash: None,
        blockheight: tx.block_height.map(|h| h as u32),
    })
}

async fn fetch_address_page(
    client: &Client,
    coin: CoinId,
    address: &str,
    limit: u32,
    offset: u32,
) -> Result<AddressDetail> {
    let url = format!(
        "{}/address/{}?limit={}&offset={}",
        api_base(coin),
        address.trim(),
        limit.max(1),
        offset
    );
    client
        .get(url)
        .send()
        .await
        .map_err(|e| WalletError::other(format!("explorer address fetch: {e}")))?
        .error_for_status()
        .map_err(|e| WalletError::other(format!("explorer address http: {e}")))?
        .json()
        .await
        .map_err(|e| WalletError::other(format!("explorer address json: {e}")))
}

async fn fetch_address_history_all(
    client: &Client,
    coin: CoinId,
    address: &str,
) -> Result<Vec<AddressTx>> {
    let first = fetch_address_page(client, coin, address, PAGE_SIZE, 0).await?;
    if !first.found {
        return Ok(Vec::new());
    }

    let mut all = first.transactions;
    let mut offset = all.len() as u32;
    let total = first
        .paging
        .as_ref()
        .and_then(|p| p.total)
        .or(first.balance.as_ref().and_then(|b| b.tx_count))
        .unwrap_or(all.len() as u64);

    while (offset as u64) < total {
        let page = fetch_address_page(client, coin, address, PAGE_SIZE, offset).await?;
        if page.transactions.is_empty() {
            break;
        }
        let fetched = page.transactions.len() as u32;
        all.extend(page.transactions);
        offset += fetched;

        let has_more = page
            .paging
            .as_ref()
            .and_then(|p| p.has_more)
            .unwrap_or((offset as u64) < total);
        if !has_more {
            break;
        }
        if fetched < PAGE_SIZE {
            break;
        }
    }

    Ok(all)
}

#[derive(Copy, Clone)]
pub struct ExplorerHistorySource;

#[async_trait::async_trait]
impl HistorySource for ExplorerHistorySource {
    async fn fetch_wallet_history(
        &self,
        coin: CoinId,
        addresses: &[String],
        _limit: usize,
        tip: Option<u32>,
    ) -> Result<Vec<WalletTx>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let client = http_client()?;
        let unique: Vec<String> = {
            let mut seen = HashSet::new();
            addresses
                .iter()
                .filter_map(|addr| {
                    let clean = addr.trim();
                    if clean.is_empty() {
                        return None;
                    }
                    if seen.insert(clean.to_string()) {
                        Some(clean.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        };

        let mut rows: Vec<WalletTx> = Vec::new();
        let mut seen_keys = HashSet::new();

        for address in unique {
            let txs = match fetch_address_history_all(&client, coin, &address).await {
                Ok(txs) => txs,
                Err(_) => continue,
            };
            for tx in &txs {
                if let Some(row) = row_from_address_tx(&address, tx, tip) {
                    let key = wallet_tx_row_key(&row);
                    if seen_keys.insert(key) {
                        rows.push(row);
                    }
                }
            }
        }

        rows.sort_by(|a, b| {
            let ta = a.time.unwrap_or(0);
            let tb = b.time.unwrap_or(0);
            tb.cmp(&ta)
                .then_with(|| b.txid.cmp(&a.txid))
                .then_with(|| b.category.cmp(&a.category))
        });
        Ok(rows)
    }
}
