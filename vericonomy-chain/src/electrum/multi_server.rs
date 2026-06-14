//! Cross-server tip verification for light wallet hardening.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::connection::{ElectrumConnection, ElectrumServerEndpoint};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipVerifyResult {
    pub consistent: bool,
    pub tips: Vec<ServerTip>,
    pub max_drift_blocks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTip {
    pub server: String,
    pub height: Option<u32>,
    pub error: Option<String>,
}

pub async fn verify_tip_consistency(
    coin: CoinId,
    server_uris: &[String],
    max_drift: u32,
) -> Result<TipVerifyResult> {
    let mut tips = Vec::new();
    for uri in server_uris {
        let endpoint = match ElectrumServerEndpoint::parse(uri) {
            Ok(ep) => ep,
            Err(e) => {
                tips.push(ServerTip {
                    server: uri.clone(),
                    height: None,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };
        match ElectrumConnection::connect(endpoint.clone()).await {
            Ok(conn) => {
                let header = conn
                    .call("blockchain.headers.subscribe", json!([]))
                    .await
                    .ok();
                let height = header
                    .as_ref()
                    .and_then(|h| h.get("height"))
                    .and_then(|v| v.as_u64())
                    .map(|h| h as u32);
                tips.push(ServerTip {
                    server: endpoint.display(),
                    height,
                    error: if height.is_none() {
                        Some(format!(
                            "no tip from {} electrum server",
                            coin.as_str()
                        ))
                    } else {
                        None
                    },
                });
            }
            Err(e) => tips.push(ServerTip {
                server: endpoint.display(),
                height: None,
                error: Some(e.to_string()),
            }),
        }
    }

    let heights: Vec<u32> = tips.iter().filter_map(|t| t.height).collect();
    let max_drift_blocks = if heights.is_empty() {
        u32::MAX
    } else {
        let min = *heights.iter().min().unwrap();
        let max = *heights.iter().max().unwrap();
        max.saturating_sub(min)
    };
    Ok(TipVerifyResult {
        consistent: !heights.is_empty() && max_drift_blocks <= max_drift,
        tips,
        max_drift_blocks,
    })
}
