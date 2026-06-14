//! Chain and network parameters shared across Vericonomy wallet clients.
//!
//! Values mirror `chainparams.cpp` in vericoin/verium. Shell-specific concerns
//! (datadir paths, explorer URLs) stay in the desktop/mobile app.

use serde::{Deserialize, Serialize};
use vericonomy_errors::WalletError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoinId {
    Verium,
    Vericoin,
}

impl CoinId {
    pub fn all() -> &'static [CoinId] {
        &[CoinId::Verium, CoinId::Vericoin]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CoinId::Verium => "verium",
            CoinId::Vericoin => "vericoin",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            CoinId::Verium => "VRM",
            CoinId::Vericoin => "VRC",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            CoinId::Verium => "Verium",
            CoinId::Vericoin => "Vericoin",
        }
    }

    pub fn profile(self) -> CoinProfile {
        CoinProfile::for_coin(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Mainnet,
    BinaryTest,
}

impl NetworkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkMode::Mainnet => "mainnet",
            NetworkMode::BinaryTest => "binarytest",
        }
    }

    pub fn is_test(self) -> bool {
        matches!(self, NetworkMode::BinaryTest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoinTarget {
    pub coin: CoinId,
    pub network: NetworkMode,
}

impl CoinTarget {
    pub fn new(coin: CoinId, network: NetworkMode) -> Self {
        Self { coin, network }
    }

    pub fn mainnet(coin: CoinId) -> Self {
        Self::new(coin, NetworkMode::Mainnet)
    }

    pub fn rpc_port(&self) -> u16 {
        match (self.coin, self.network) {
            (CoinId::Verium, NetworkMode::Mainnet) => 33987,
            (CoinId::Vericoin, NetworkMode::Mainnet) => 58683,
            (CoinId::Verium, NetworkMode::BinaryTest) => 41987,
            (CoinId::Vericoin, NetworkMode::BinaryTest) => 41683,
        }
    }
}

/// Cryptographic and network constants for a single chain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CoinProfile {
    pub coin: CoinId,
    pub p2pkh_version: u8,
    pub wif_secret_prefix: u8,
    pub bech32_hrp: &'static str,
    pub bip44_coin_type: u32,
    pub maturity_confirmations: u32,
    pub default_rpc_port: u16,
    pub default_p2p_port: u16,
    pub earn_mode: &'static str,
    pub currency_symbol: &'static str,
    /// Display prefix for P2PKH addresses (Base58Check version 70 → `V`).
    pub p2pkh_address_prefix: &'static str,
    pub p2pkh_address_length: u32,
}

impl CoinProfile {
    pub fn for_coin(coin: CoinId) -> Self {
        match coin {
            CoinId::Verium => Self {
                coin,
                p2pkh_version: 70,
                wif_secret_prefix: 198,
                bech32_hrp: "vry",
                bip44_coin_type: 462,
                maturity_confirmations: 100,
                default_rpc_port: 33987,
                default_p2p_port: 36988,
                earn_mode: "mining",
                currency_symbol: "VRM",
                p2pkh_address_prefix: "V",
                p2pkh_address_length: 34,
            },
            CoinId::Vericoin => Self {
                coin,
                p2pkh_version: 70,
                wif_secret_prefix: 198,
                bech32_hrp: "vry",
                bip44_coin_type: 463,
                maturity_confirmations: 500,
                default_rpc_port: 58683,
                default_p2p_port: 58684,
                earn_mode: "staking",
                currency_symbol: "VRC",
                p2pkh_address_prefix: "V",
                p2pkh_address_length: 34,
            },
        }
    }

    pub fn coin_type(&self) -> u32 {
        self.bip44_coin_type
    }

    pub fn default_electrum_servers(&self, network: NetworkMode) -> Vec<String> {
        if network.is_test() {
            return match self.coin {
                CoinId::Verium => vec!["tls://electrumx-vrm3.vericonomy.com:53002".into()],
                CoinId::Vericoin => vec!["tls://electrumx-vrc3.vericonomy.com:53012".into()],
            };
        }
        let env_key = match self.coin {
            CoinId::Verium => "VERICONOMY_VRM_ELECTRUM_SERVERS",
            CoinId::Vericoin => "VERICONOMY_VRC_ELECTRUM_SERVERS",
        };
        if let Ok(raw) = std::env::var(env_key) {
            let servers: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !servers.is_empty() {
                return servers;
            }
        }
        match self.coin {
            CoinId::Verium => vec![
                "tls://electrumx-vrm3.vericonomy.com:53002".into(),
                "tls://electrumx-vrm1.vericonomy.com:51002".into(),
                "tls://electrumx-vrm2.vericonomy.com:52002".into(),
            ],
            CoinId::Vericoin => vec![
                "tls://electrumx-vrc3.vericonomy.com:53012".into(),
                "tls://electrumx-vrc1.vericonomy.com:50012".into(),
                "tls://electrumx-vrc2.vericonomy.com:50012".into(),
            ],
        }
    }
}

/// JSON schema consumed by TypeScript shells (`coin-profiles.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinProfileJson {
    pub id: String,
    pub symbol: String,
    pub display_name: String,
    pub p2pkh_version: u8,
    pub wif_secret_prefix: u8,
    pub bech32_hrp: String,
    pub bip44_coin_type: u32,
    pub maturity_confirmations: u32,
    pub default_rpc_port: u16,
    pub default_p2p_port: u16,
    pub earn_mode: String,
    pub p2pkh_address_prefix: String,
    pub p2pkh_address_length: u32,
}

pub fn all_profiles_json() -> Vec<CoinProfileJson> {
    CoinId::all()
        .iter()
        .map(|&coin| {
            let p = coin.profile();
            CoinProfileJson {
                id: coin.as_str().to_string(),
                symbol: p.currency_symbol.to_string(),
                display_name: coin.display_name().to_string(),
                p2pkh_version: p.p2pkh_version,
                wif_secret_prefix: p.wif_secret_prefix,
                bech32_hrp: p.bech32_hrp.to_string(),
                bip44_coin_type: p.bip44_coin_type,
                maturity_confirmations: p.maturity_confirmations,
                default_rpc_port: p.default_rpc_port,
                default_p2p_port: p.default_p2p_port,
                earn_mode: p.earn_mode.to_string(),
                p2pkh_address_prefix: p.p2pkh_address_prefix.to_string(),
                p2pkh_address_length: p.p2pkh_address_length,
            }
        })
        .collect()
}

pub fn profiles_json_string() -> Result<String, WalletError> {
    serde_json::to_string_pretty(&all_profiles_json()).map_err(|e| WalletError::Serde {
        code: WalletError::CODE_SERDE,
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_chains_share_p2pkh_and_wif_prefixes() {
        for coin in CoinId::all() {
            let p = coin.profile();
            assert_eq!(p.p2pkh_version, 70);
            assert_eq!(p.wif_secret_prefix, 198);
        }
    }

    #[test]
    fn bip44_coin_types_match_slip44() {
        assert_eq!(CoinId::Verium.profile().bip44_coin_type, 462);
        assert_eq!(CoinId::Vericoin.profile().bip44_coin_type, 463);
    }
}
