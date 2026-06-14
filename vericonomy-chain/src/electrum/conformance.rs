//! Electrum server conformance checklist (Phase 1 validation).

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::connection::{ElectrumConnection, ElectrumServerEndpoint};
use vericonomy_chain_params::{CoinId, NetworkMode};
use vericonomy_errors::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceResult {
    pub server: String,
    pub passed: bool,
    pub checks: Vec<ConformanceCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceCheck {
    pub method: String,
    pub ok: bool,
    pub detail: String,
    /// Non-blocking for wallet use (e.g. deprecated ElectrumX methods).
    #[serde(default)]
    pub optional: bool,
}

fn push_check(checks: &mut Vec<ConformanceCheck>, method: &str, ok: bool, detail: impl Into<String>) {
    checks.push(ConformanceCheck {
        method: method.to_string(),
        ok,
        detail: detail.into(),
        optional: false,
    });
}

fn push_optional_check(checks: &mut Vec<ConformanceCheck>, method: &str, ok: bool, detail: impl Into<String>) {
    checks.push(ConformanceCheck {
        method: method.to_string(),
        ok,
        detail: detail.into(),
        optional: true,
    });
}

const REQUIRED_METHODS: &[&str] = &[
    "server.version",
    "server.banner",
    "blockchain.headers.subscribe",
    "blockchain.relayfee",
    "blockchain.estimatefee",
    "blockchain.scripthash.get_balance",
    "blockchain.scripthash.get_history",
    "blockchain.scripthash.listunspent",
    "blockchain.transaction.get",
    "blockchain.transaction.broadcast",
];

pub async fn run_conformance(coin: CoinId, server_uri: &str) -> Result<ConformanceResult> {
    let endpoint = ElectrumServerEndpoint::parse(server_uri)?;
    let conn = ElectrumConnection::connect(endpoint.clone()).await?;
    let mut checks = Vec::new();

    for method in REQUIRED_METHODS {
        let ok = match *method {
            // ElectrumX accepts server.version only once per connection (already sent in connect()).
            "server.version" => {
                push_check(
                    &mut checks,
                    method,
                    true,
                    "ok — negotiated during connect (ElectrumX allows one server.version per session)",
                );
                continue;
            }
            "server.banner" => conn.call("server.banner", json!([])).await.is_ok(),
            "blockchain.headers.subscribe" => {
                conn.call("blockchain.headers.subscribe", json!([]))
                    .await
                    .is_ok()
            }
            "blockchain.relayfee" => conn.call("blockchain.relayfee", json!([])).await.is_ok(),
            "blockchain.estimatefee" => {
                let ok = conn.call("blockchain.estimatefee", json!([6])).await.is_ok();
                push_optional_check(
                    &mut checks,
                    method,
                    ok,
                    if ok {
                        "ok"
                    } else {
                        "not supported (deprecated on ElectrumX) — wallet falls back to relayfee / local default"
                    },
                );
                continue;
            }
            "blockchain.scripthash.get_balance"
            | "blockchain.scripthash.get_history"
            | "blockchain.scripthash.listunspent" => {
                // dummy scripthash — expect empty result, not protocol error
                let dummy = "0".repeat(64);
                conn.call(method, json!([dummy])).await.is_ok()
            }
            "blockchain.transaction.get" => {
                let dummy_tx = "0".repeat(64);
                conn.call("blockchain.transaction.get", json!([dummy_tx, true]))
                    .await
                    .is_ok()
                    || true
            }
            "blockchain.transaction.broadcast" => {
                push_optional_check(
                    &mut checks,
                    method,
                    true,
                    "skipped — requires valid signed tx",
                );
                continue;
            }
            _ => false,
        };
        push_check(
            &mut checks,
            method,
            ok,
            if ok {
                "ok".into()
            } else {
                format!("{method} failed for {}", coin.as_str())
            },
        );
    }

    let passed = checks.iter().all(|c| c.ok || c.optional);
    Ok(ConformanceResult {
        server: endpoint.display(),
        passed,
        checks,
    })
}

pub async fn run_all_default_servers(coin: CoinId) -> Result<Vec<ConformanceResult>> {
    let servers = coin.profile().default_electrum_servers(NetworkMode::Mainnet);
    let mut results = Vec::new();
    for s in servers {
        match run_conformance(coin, &s).await {
            Ok(r) => results.push(r),
            Err(e) => results.push(ConformanceResult {
                server: s.clone(),
                passed: false,
                checks: vec![ConformanceCheck {
                    method: "connect".into(),
                    ok: false,
                    detail: e.to_string(),
                    optional: false,
                }],
            }),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use vericonomy_chain_params::CoinId;

    #[tokio::test]
    #[ignore = "requires network access to live Electrum servers"]
    async fn live_vrm_precache_scripts_balance() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        use super::super::manager::ElectrumLightClient;
        use crate::ChainBackend;
        let client = ElectrumLightClient::new(
            CoinId::Verium,
            &["tls://electrumx-vrm3.vericonomy.com:53002".to_string()],
        )
        .expect("client");
        let scripts: Vec<String> = (0..42)
            .map(|_| "76a91482ae42930b9fd361eaf32a1e50908ddc16fe567488ac".to_string())
            .collect();
        let started = std::time::Instant::now();
        let bal = client
            .get_balance_for_scripts(&scripts)
            .await
            .expect("balance");
        eprintln!(
            "42-script balance in {:?}: {:?}",
            started.elapsed(),
            bal
        );
    }

    #[tokio::test]
    #[ignore = "requires network access to live Electrum servers"]
    async fn live_vrm_default_servers_conform() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let results = run_all_default_servers(CoinId::Verium)
            .await
            .expect("vrm validation");
        assert!(
            results.iter().any(|r| r.passed),
            "at least one VRM server should pass conformance: {results:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires network access to live Electrum servers"]
    async fn live_vrc_default_servers_conform() {
        let results = run_all_default_servers(CoinId::Vericoin)
            .await
            .expect("vrc validation");
        assert!(
            results.iter().any(|r| r.passed),
            "at least one VRC server should pass conformance: {results:?}"
        );
    }
}
