//! Portable wallet core: BIP39 mnemonics, BIP32 derivation, WIF encoding.

mod error;
pub mod recovery;

pub use error::WalletCoreError;
pub use recovery::{
    derive_account_wif, derive_master_xpriv, generate_mnemonic, master_xpriv_to_wif,
    secret_bytes_to_wif, validate_mnemonic, verification_indices, verify_words_at_indices,
    zeroize_string, RecoveryPhraseBundle,
};
