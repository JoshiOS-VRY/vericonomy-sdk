//! UniFFI entry point for mobile shells (iOS first).

use std::sync::OnceLock;

use vericonomy_chain::electrum::ElectrumLightClient;
use vericonomy_chain::ChainBackend;
use vericonomy_chain_params::{CoinId, NetworkMode};
use vericonomy_errors::WalletError;
use vericonomy_hd::{address_to_script_pubkey, derive_address_at};
use vericonomy_wallet_core::{generate_mnemonic, validate_mnemonic, WalletCoreError};
use vericonomy_wallet_engine::validate_send_address;

uniffi::setup_scaffolding!();

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiWalletError {
    #[error("{message}")]
    Wallet { code: u32, message: String },
}

impl From<WalletError> for FfiWalletError {
    fn from(e: WalletError) -> Self {
        Self::Wallet {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

impl From<WalletCoreError> for FfiWalletError {
    fn from(e: WalletCoreError) -> Self {
        Self::Wallet {
            code: WalletError::CODE_OTHER,
            message: e.to_string(),
        }
    }
}

#[derive(uniffi::Record)]
pub struct BalanceInfo {
    pub confirmed_sats: i64,
    pub unconfirmed_sats: i64,
    pub total_sats: i64,
}

fn parse_coin(coin: &str) -> Result<CoinId, FfiWalletError> {
    match coin.trim().to_ascii_lowercase().as_str() {
        "verium" | "vrm" => Ok(CoinId::Verium),
        "vericoin" | "vrc" => Ok(CoinId::Vericoin),
        other => Err(FfiWalletError::Wallet {
            code: WalletError::CODE_OTHER,
            message: format!("unknown coin: {other}"),
        }),
    }
}

/// Mobile wallet API surface (UniFFI → Swift).
#[derive(uniffi::Object)]
pub struct WalletFacade;

#[uniffi::export]
impl WalletFacade {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }

    pub fn validate_mnemonic(&self, phrase: String) -> Result<bool, FfiWalletError> {
        Ok(validate_mnemonic(&phrase))
    }

    pub fn generate_mnemonic(&self) -> Result<String, FfiWalletError> {
        let bundle = generate_mnemonic().map_err(FfiWalletError::from)?;
        Ok(bundle.mnemonic.clone())
    }

    pub fn derive_address(
        &self,
        coin: String,
        mnemonic: String,
        index: u32,
    ) -> Result<String, FfiWalletError> {
        let coin = parse_coin(&coin)?;
        derive_address_at(coin, &mnemonic, None, index).map_err(FfiWalletError::from)
    }

    pub fn validate_send_address(&self, coin: String, address: String) -> Result<(), FfiWalletError> {
        let coin = parse_coin(&coin)?;
        validate_send_address(coin, &address).map_err(FfiWalletError::from)
    }

    pub fn default_electrum_servers(&self, coin: String) -> Result<Vec<String>, FfiWalletError> {
        let coin = parse_coin(&coin)?;
        Ok(coin
            .profile()
            .default_electrum_servers(NetworkMode::Mainnet))
    }

    /// Scan receive addresses `0..=max_index` against Electrum and return aggregate balance.
    pub fn get_light_balance(
        &self,
        coin: String,
        mnemonic: String,
        max_index: u32,
    ) -> Result<BalanceInfo, FfiWalletError> {
        let coin = parse_coin(&coin)?;
        let servers = coin.profile().default_electrum_servers(NetworkMode::Mainnet);
        let max_index = max_index.min(80);

        runtime()
            .block_on(async move {
                let client = ElectrumLightClient::new(coin, &servers)?;
                let mut scripts = Vec::new();
                for i in 0..=max_index {
                    let addr = derive_address_at(coin, &mnemonic, None, i)?;
                    let script = address_to_script_pubkey(coin, &addr)?;
                    scripts.push(hex::encode(script));
                }
                let balance = client.get_balance_for_scripts(&scripts).await?;
                Ok::<BalanceInfo, WalletError>(BalanceInfo {
                    confirmed_sats: balance.confirmed_sats,
                    unconfirmed_sats: balance.unconfirmed_sats,
                    total_sats: balance.total_sats(),
                })
            })
            .map_err(FfiWalletError::from)
    }
}
