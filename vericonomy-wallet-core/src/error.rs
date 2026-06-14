use thiserror::Error;

/// Errors produced by the portable wallet core. The desktop shell maps these
/// into its own `AppError`; mobile shells can surface them directly.
#[derive(Debug, Error)]
pub enum WalletCoreError {
    #[error("mnemonic generation failed: {0}")]
    MnemonicGeneration(String),
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
    #[error("master key derivation failed: {0}")]
    MasterKeyDerivation(String),
    #[error("invalid derivation path: {0}")]
    DerivationPath(String),
    #[error("key derivation failed: {0}")]
    Derive(String),
}

pub type Result<T> = std::result::Result<T, WalletCoreError>;
