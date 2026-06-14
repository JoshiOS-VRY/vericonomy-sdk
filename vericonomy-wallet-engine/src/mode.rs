//! Wallet operating mode: full local node vs Electrum light client.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WalletMode {
    #[default]
    FullNode,
    Light,
}

impl WalletMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WalletMode::FullNode => "full_node",
            WalletMode::Light => "light",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" | "light_wallet" => WalletMode::Light,
            _ => WalletMode::FullNode,
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, WalletMode::Light)
    }
}
