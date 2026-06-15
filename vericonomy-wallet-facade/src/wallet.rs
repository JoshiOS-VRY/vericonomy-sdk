//! Primary light-wallet handle for shells.

use vericonomy_chain::ChainBackend;
use vericonomy_chain::types::{Utxo, WalletBalance, WalletTx};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd::{
    address_to_script_pubkey, coins_to_sats, derive_address_at, derive_change_address_at,
    sats_to_coins, uses_core_hd_paths, GAP_SCAN_MAX_INDEX,
};
use vericonomy_wallet_core::{generate_mnemonic, validate_mnemonic, RecoveryPhraseBundle};
use vericonomy_wallet_engine::{
    validate_send_address, DEFAULT_TX_FEE_COINS_PER_KB, SignedTx,
};
use vericonomy_wallet_engine::signer::{build_unsigned_hex, sign_transaction};

use crate::balance::{balance_from_utxos, spendable_utxos};
use crate::send::{send_payment, SendPaymentParams, SendResult};
use crate::utxo_prep::prepare_utxos_for_signing;

/// Coin-scoped light wallet bound to a chain backend (Electrum or full node).
pub struct LightWallet<B> {
    coin: CoinId,
    backend: B,
}

impl<B: ChainBackend> LightWallet<B> {
    pub fn new(coin: CoinId, backend: B) -> Self {
        Self { coin, backend }
    }

    pub fn coin(&self) -> CoinId {
        self.coin
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    // --- Recovery / addresses (sync, no backend) ---

    pub fn validate_mnemonic(phrase: &str) -> bool {
        validate_mnemonic(phrase)
    }

    pub fn generate_mnemonic() -> Result<RecoveryPhraseBundle> {
        generate_mnemonic().map_err(|e| WalletError::other(e.to_string()))
    }

    pub fn derive_receive_address(
        &self,
        mnemonic: &str,
        bip39_passphrase: Option<&str>,
        index: u32,
    ) -> Result<String> {
        derive_address_at(self.coin, mnemonic, bip39_passphrase, index)
    }

    pub fn derive_change_address(
        &self,
        mnemonic: &str,
        bip39_passphrase: Option<&str>,
        index: u32,
    ) -> Result<String> {
        derive_change_address_at(self.coin, mnemonic, bip39_passphrase, index)
    }

    pub fn resolve_change_address(
        &self,
        mnemonic: &str,
        bip39_passphrase: Option<&str>,
        change_index: u32,
    ) -> Result<String> {
        if uses_core_hd_paths(self.coin, mnemonic) {
            derive_change_address_at(self.coin, mnemonic, bip39_passphrase, change_index)
        } else {
            derive_address_at(self.coin, mnemonic, bip39_passphrase, change_index)
        }
    }

    pub fn validate_send_address(&self, address: &str) -> Result<()> {
        validate_send_address(self.coin, address)
    }

    pub fn coins_to_sats(coins: f64) -> i64 {
        coins_to_sats(coins)
    }

    pub fn sats_to_coins(sats: i64) -> f64 {
        sats_to_coins(sats)
    }

    // --- Chain reads ---

    pub async fn script_hexes_for_indices(
        &self,
        mnemonic: &str,
        bip39_passphrase: Option<&str>,
        max_index: u32,
    ) -> Result<Vec<String>> {
        let max_index = max_index.min(GAP_SCAN_MAX_INDEX);
        let mut scripts = Vec::with_capacity(max_index as usize + 1);
        for i in 0..=max_index {
            let addr = derive_address_at(self.coin, mnemonic, bip39_passphrase, i)?;
            let script = address_to_script_pubkey(self.coin, &addr)?;
            scripts.push(hex::encode(script));
        }
        Ok(scripts)
    }

    pub async fn get_balance_for_scripts(&self, script_hexes: &[String]) -> Result<WalletBalance> {
        self.backend.get_balance_for_scripts(script_hexes).await
    }

    pub async fn list_utxos_for_scripts(&self, script_hexes: &[String]) -> Result<Vec<Utxo>> {
        self.backend.list_utxos_for_scripts(script_hexes).await
    }

    pub async fn list_spendable_utxos_for_scripts(
        &self,
        script_hexes: &[String],
    ) -> Result<Vec<Utxo>> {
        Ok(spendable_utxos(
            self.backend.list_utxos_for_scripts(script_hexes).await?,
        ))
    }

    pub async fn get_history_for_scripts(
        &self,
        script_hexes: &[String],
        limit: usize,
    ) -> Result<Vec<WalletTx>> {
        self.backend
            .get_history_for_scripts(script_hexes, limit)
            .await
    }

    pub async fn get_light_balance(
        &self,
        mnemonic: &str,
        bip39_passphrase: Option<&str>,
        max_index: u32,
    ) -> Result<WalletBalance> {
        let scripts = self
            .script_hexes_for_indices(mnemonic, bip39_passphrase, max_index)
            .await?;
        self.get_balance_for_scripts(&scripts).await
    }

    pub async fn balance_from_utxo_set(&self, utxos: &[Utxo]) -> Result<WalletBalance> {
        let tip = self.backend.get_tip().await?;
        Ok(balance_from_utxos(utxos, tip.height))
    }

    // --- Send ---

    pub async fn send_to_address(
        &self,
        mnemonic: &str,
        recipient: &str,
        amount_sats: i64,
        fee_rate_coins_per_kb: Option<f64>,
        change_address: &str,
        utxos: Vec<Utxo>,
        bip39_passphrase: Option<&str>,
    ) -> Result<SendResult> {
        send_payment(
            &self.backend,
            self.coin,
            mnemonic,
            SendPaymentParams {
                recipient: recipient.to_string(),
                amount_sats,
                fee_rate_coins_per_kb: fee_rate_coins_per_kb
                    .unwrap_or(DEFAULT_TX_FEE_COINS_PER_KB),
                utxos,
                change_address: change_address.to_string(),
                bip39_passphrase: bip39_passphrase.map(str::to_string),
            },
        )
        .await
    }

    pub async fn sign_payment(
        &self,
        mnemonic: &str,
        utxos: Vec<Utxo>,
        outputs: &[(String, i64)],
        change_address: &str,
        fee_rate_coins_per_kb: f64,
        bip39_passphrase: Option<&str>,
    ) -> Result<SignedTx> {
        let rate = if fee_rate_coins_per_kb > 0.0 {
            fee_rate_coins_per_kb
        } else {
            DEFAULT_TX_FEE_COINS_PER_KB
        };
        let total_out: i64 = outputs.iter().map(|(_, v)| *v).sum();
        let (mut selected, _) = vericonomy_wallet_engine::utxo_selector::plan_send_utxos(
            &utxos,
            total_out,
            rate,
            outputs.len(),
        )?;
        prepare_utxos_for_signing(&self.backend, &mut selected).await?;
        let fee_sats =
            vericonomy_wallet_engine::utxo_selector::replan_fee_for_selected(
                &selected,
                total_out,
                rate,
                outputs.len(),
            )?;
        sign_transaction(
            self.coin,
            mnemonic,
            bip39_passphrase,
            &selected,
            outputs,
            change_address,
            fee_sats,
        )
    }

    pub async fn build_unsigned_payment(
        &self,
        mnemonic: &str,
        utxos: Vec<Utxo>,
        outputs: &[(String, i64)],
        change_address: &str,
        fee_rate_coins_per_kb: f64,
        bip39_passphrase: Option<&str>,
    ) -> Result<String> {
        let rate = if fee_rate_coins_per_kb > 0.0 {
            fee_rate_coins_per_kb
        } else {
            DEFAULT_TX_FEE_COINS_PER_KB
        };
        let total_out: i64 = outputs.iter().map(|(_, v)| *v).sum();
        let (mut selected, _) = vericonomy_wallet_engine::utxo_selector::plan_send_utxos(
            &utxos,
            total_out,
            rate,
            outputs.len(),
        )?;
        prepare_utxos_for_signing(&self.backend, &mut selected).await?;
        let fee_sats =
            vericonomy_wallet_engine::utxo_selector::replan_fee_for_selected(
                &selected,
                total_out,
                rate,
                outputs.len(),
            )?;
        build_unsigned_hex(
            self.coin,
            mnemonic,
            bip39_passphrase,
            &selected,
            outputs,
            change_address,
            fee_sats,
        )
    }
}
