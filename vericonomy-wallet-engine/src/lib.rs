//! Wallet engine: UTXO selection, transaction signing, address validation.

pub mod address;
pub mod mode;
pub mod signer;
pub mod utxo_selector;

pub use address::validate_send_address;
pub use mode::WalletMode;
pub use signer::{build_unsigned_hex, sign_transaction, SignedTx};
pub use utxo_selector::{
    coins_per_kb_to_sats_per_k, fee_for_rate, plan_send_utxos, replan_fee_for_selected,
    select_utxos, DEFAULT_TX_FEE_COINS_PER_KB, DUST_CHANGE_SATS, VIP1_MIN_TX_FEE_PER_K,
};
