//! UniFFI entry point for native iOS wallet (SwiftUI + `LightWalletSession`).

use std::path::PathBuf;
use std::sync::Arc;

use vericonomy_chain::types::{Utxo, WalletBalance, WalletTx};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::WalletError;
use vericonomy_hd::{coins_to_sats, sats_to_coins};
use vericonomy_storage_ios::{
    open_ios_wallet_stores, IosFileKeystoreStore, IosSqliteCache, IosWalletStores,
};
use vericonomy_wallet_engine::DEFAULT_TX_FEE_COINS_PER_KB;
use vericonomy_wallet_facade::{LightWallet, LightWalletSession, ExplorerHistorySource};

static HISTORY: ExplorerHistorySource = ExplorerHistorySource;

uniffi::setup_scaffolding!();

static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    vericonomy_chain::ensure_tls_crypto_provider();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    runtime().block_on(f)
}

type Session = LightWalletSession<IosFileKeystoreStore, IosSqliteCache, IosSqliteCache>;

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

#[derive(uniffi::Record)]
pub struct BalanceInfo {
    pub confirmed_sats: i64,
    pub unconfirmed_sats: i64,
    pub total_sats: i64,
}

#[derive(uniffi::Record)]
pub struct SendResultInfo {
    pub txid: String,
    pub raw_hex: String,
    pub fee_sats: i64,
    pub change_sats: i64,
}

#[derive(uniffi::Record)]
pub struct UtxoInfo {
    pub txid: String,
    pub vout: u32,
    pub value_sats: i64,
    pub address: String,
    pub script_hex: String,
    pub confirmations: u32,
}

#[derive(uniffi::Record)]
pub struct WalletTxInfo {
    pub txid: String,
    pub height: i32,
    pub category: String,
    pub amount: f64,
    pub address: Option<String>,
    pub confirmations: i32,
    pub time: Option<u64>,
}

#[derive(uniffi::Record)]
pub struct WalletStatusInfo {
    pub scan_phase: String,
    pub scan_complete: bool,
    pub sync_in_flight: bool,
    pub balance_ready: bool,
    pub unlocked: bool,
    pub wallet_exists: bool,
}

#[derive(uniffi::Record)]
pub struct SyncReportInfo {
    pub scan_complete: bool,
    pub balance_confirmed_sats: i64,
    pub balance_unconfirmed_sats: i64,
}

#[derive(uniffi::Record)]
pub struct LightServerStatusInfo {
    pub connected: bool,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub latency_ms: Option<u64>,
    pub tip_height: Option<u32>,
    pub servers_total: u32,
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

fn balance_info(balance: WalletBalance) -> BalanceInfo {
    BalanceInfo {
        confirmed_sats: balance.confirmed_sats,
        unconfirmed_sats: balance.unconfirmed_sats,
        total_sats: balance.total_sats(),
    }
}

fn utxo_info(utxo: Utxo) -> UtxoInfo {
    UtxoInfo {
        txid: utxo.txid,
        vout: utxo.vout,
        value_sats: utxo.value_sats,
        address: utxo.address,
        script_hex: utxo.script_hex,
        confirmations: utxo.confirmations,
    }
}

fn tx_info(tx: WalletTx) -> WalletTxInfo {
    WalletTxInfo {
        txid: tx.txid,
        height: tx.height,
        category: tx.category,
        amount: tx.amount,
        address: tx.address,
        confirmations: tx.confirmations,
        time: tx.time,
    }
}

fn session_from_stores(coin: CoinId, stores: IosWalletStores) -> Result<Session, WalletError> {
    let IosWalletStores {
        keystore,
        utxo_cache,
        tx_cache,
    } = stores;
    LightWalletSession::with_default_servers(
        coin,
        vericonomy_storage::LightKeystoreService::new(keystore),
        utxo_cache,
        tx_cache,
    )
}

/// Coin-scoped light wallet session — primary iOS API surface.
#[derive(uniffi::Object)]
pub struct LightWalletSessionHandle {
    inner: Arc<Session>,
    coin: CoinId,
}

#[uniffi::export]
impl LightWalletSessionHandle {
    #[uniffi::constructor]
    pub fn new(coin: String, storage_dir: Option<String>) -> Result<Arc<Self>, FfiWalletError> {
        let coin = parse_coin(&coin)?;
        let base = storage_dir.map(PathBuf::from);
        let stores = open_ios_wallet_stores(base);
        let session = session_from_stores(coin, stores).map_err(FfiWalletError::from)?;
        Ok(Arc::new(Self {
            inner: Arc::new(session),
            coin,
        }))
    }

    pub fn validate_mnemonic(&self, phrase: String) -> Result<bool, FfiWalletError> {
        Ok(LightWallet::<vericonomy_chain::electrum::ElectrumLightClient>::validate_mnemonic(
            &phrase,
        ))
    }

    pub fn generate_mnemonic(&self) -> Result<String, FfiWalletError> {
        let bundle =
            LightWallet::<vericonomy_chain::electrum::ElectrumLightClient>::generate_mnemonic()?;
        Ok(bundle.mnemonic.clone())
    }

    pub fn default_electrum_servers(&self) -> Vec<String> {
        self.inner.default_electrum_servers()
    }

    pub fn configured_electrum_servers(&self) -> Result<Vec<String>, FfiWalletError> {
        Ok(block_on(self.inner.configured_electrum_servers()))
    }

    pub fn set_electrum_servers(&self, servers: Vec<String>) -> Result<(), FfiWalletError> {
        block_on(self.inner.set_electrum_servers(servers)).map_err(FfiWalletError::from)
    }

    pub fn create_wallet(
        &self,
        mnemonic: String,
        passphrase: String,
        label: Option<String>,
    ) -> Result<(), FfiWalletError> {
        block_on(self.inner.create_wallet(
            &mnemonic,
            &passphrase,
            label.as_deref(),
        ))
        .map_err(FfiWalletError::from)
    }

    pub fn import_wallet(
        &self,
        seed_secret: String,
        passphrase: String,
        label: Option<String>,
    ) -> Result<(), FfiWalletError> {
        block_on(self.inner.import_wallet(
            &seed_secret,
            &passphrase,
            label.as_deref(),
        ))
        .map_err(FfiWalletError::from)
    }

    pub fn unlock(&self, passphrase: String, seconds: u32) -> Result<(), FfiWalletError> {
        block_on(self.inner.unlock(&passphrase, seconds)).map_err(FfiWalletError::from)
    }

    pub fn lock(&self) -> Result<(), FfiWalletError> {
        block_on(self.inner.lock()).map_err(FfiWalletError::from)
    }

    pub fn exists(&self) -> Result<bool, FfiWalletError> {
        block_on(self.inner.exists()).map_err(FfiWalletError::from)
    }

    pub fn status(&self) -> Result<WalletStatusInfo, FfiWalletError> {
        block_on(async {
            let status = self.inner.status().await?;
            Ok::<WalletStatusInfo, WalletError>(WalletStatusInfo {
                scan_phase: format!("{:?}", status.scan_phase).to_ascii_lowercase(),
                scan_complete: status.scan_complete,
                sync_in_flight: status.sync_in_flight,
                balance_ready: status.balance_ready,
                unlocked: status.unlocked,
                wallet_exists: status.wallet_exists,
            })
        })
        .map_err(FfiWalletError::from)
    }

    pub fn sync(&self) -> Result<SyncReportInfo, FfiWalletError> {
        block_on(async {
            let history = HISTORY;
            let report = self.inner.sync(&history).await?;
            Ok::<SyncReportInfo, WalletError>(SyncReportInfo {
                scan_complete: report.scan_complete,
                balance_confirmed_sats: report.balance_confirmed_sats,
                balance_unconfirmed_sats: report.balance_unconfirmed_sats,
            })
        })
        .map_err(FfiWalletError::from)
    }

    pub fn refresh_balance(&self) -> Result<(), FfiWalletError> {
        block_on(self.inner.refresh_balance()).map_err(FfiWalletError::from)
    }

    pub fn refresh_pending(&self) -> Result<(), FfiWalletError> {
        block_on(async {
            let history = HISTORY;
            self.inner.refresh_pending(&history).await
        })
        .map_err(FfiWalletError::from)
    }

    pub fn rescan(&self) -> Result<(), FfiWalletError> {
        block_on(self.inner.rescan()).map_err(FfiWalletError::from)
    }

    pub fn balance(&self) -> Result<BalanceInfo, FfiWalletError> {
        block_on(async {
            let balance = self.inner.balance().await?;
            Ok::<BalanceInfo, WalletError>(balance_info(balance))
        })
        .map_err(FfiWalletError::from)
    }

    pub fn new_receive_address(&self, passphrase: String) -> Result<String, FfiWalletError> {
        block_on(self.inner.new_receive_address(&passphrase)).map_err(FfiWalletError::from)
    }

    pub fn list_receive_addresses(&self) -> Result<Vec<String>, FfiWalletError> {
        block_on(self.inner.list_receive_addresses()).map_err(FfiWalletError::from)
    }

    pub fn list_spendable_utxos(&self, passphrase: String) -> Result<Vec<UtxoInfo>, FfiWalletError> {
        block_on(async {
            let utxos = self.inner.list_spendable_utxos(&passphrase).await?;
            Ok::<Vec<UtxoInfo>, WalletError>(utxos.into_iter().map(utxo_info).collect())
        })
        .map_err(FfiWalletError::from)
    }

    pub fn list_transactions(&self, limit: u32) -> Result<Vec<WalletTxInfo>, FfiWalletError> {
        block_on(async {
            let history = HISTORY;
            let rows = self
                .inner
                .list_transactions(&history, limit as usize)
                .await?;
            Ok::<Vec<WalletTxInfo>, WalletError>(rows.into_iter().map(tx_info).collect())
        })
        .map_err(FfiWalletError::from)
    }

    pub fn send_to_address(
        &self,
        recipient: String,
        amount_coins: f64,
        fee_rate_coins_per_kb: Option<f64>,
        passphrase: String,
    ) -> Result<SendResultInfo, FfiWalletError> {
        block_on(async {
            let history = HISTORY;
            let result = self
                .inner
                .send_to_address(
                    &history,
                    &recipient,
                    coins_to_sats(amount_coins),
                    fee_rate_coins_per_kb,
                    &passphrase,
                )
                .await?;
            Ok::<SendResultInfo, WalletError>(SendResultInfo {
                txid: result.txid,
                raw_hex: result.raw_hex,
                fee_sats: result.fee_sats,
                change_sats: result.change_sats,
            })
        })
        .map_err(FfiWalletError::from)
    }

    pub fn estimate_fee(&self, target_blocks: u32) -> Result<f64, FfiWalletError> {
        block_on(self.inner.estimate_fee(target_blocks)).map_err(FfiWalletError::from)
    }

    pub fn server_status(&self) -> Option<LightServerStatusInfo> {
        block_on(async {
            self.inner.server_status().await.map(|s| LightServerStatusInfo {
                connected: s.connected,
                server_host: s.server_host,
                server_port: s.server_port,
                latency_ms: s.latency_ms,
                tip_height: s.tip_height,
                servers_total: s.servers_total as u32,
            })
        })
    }

    pub fn coins_to_sats(&self, coins: f64) -> i64 {
        coins_to_sats(coins)
    }

    pub fn sats_to_coins(&self, sats: i64) -> f64 {
        sats_to_coins(sats)
    }

    pub fn default_fee_rate_coins_per_kb(&self) -> f64 {
        DEFAULT_TX_FEE_COINS_PER_KB
    }

    pub fn coin_id(&self) -> String {
        self.coin.as_str().to_string()
    }

    pub fn export_mnemonic(&self, passphrase: String) -> Result<String, FfiWalletError> {
        block_on(self.inner.export_mnemonic(&passphrase)).map_err(FfiWalletError::from)
    }

    pub fn watch_scripthashes(&self) -> Result<Vec<String>, FfiWalletError> {
        block_on(self.inner.watch_scripthashes()).map_err(FfiWalletError::from)
    }
}

/// Backward-compatible alias for early integrations.
pub type LightWalletEngine = LightWalletSessionHandle;
pub type WalletFacade = LightWalletSessionHandle;
