//! Shared error types for the Vericonomy wallet SDK.
//!
//! Stable `code` values are suitable for FFI (UniFFI / mobile shells).

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WalletError {
    #[error("mnemonic generation failed: {message}")]
    MnemonicGeneration { code: u32, message: String },
    #[error("invalid mnemonic: {message}")]
    InvalidMnemonic { code: u32, message: String },
    #[error("derivation failed: {message}")]
    Derivation { code: u32, message: String },
    #[error("invalid address: {message}")]
    InvalidAddress { code: u32, message: String },
    #[error("transaction error: {message}")]
    Transaction { code: u32, message: String },
    #[error("insufficient funds: {message}")]
    InsufficientFunds { code: u32, message: String },
    #[error("wallet locked")]
    LockedWallet { code: u32 },
    #[error("wrong passphrase")]
    WrongPassphrase { code: u32 },
    #[error("RPC error: {message}")]
    Rpc { code: u32, message: String },
    #[error("Electrum error: {message}")]
    Electrum { code: u32, message: String },
    #[error("HTTP error: {message}")]
    Http { code: u32, message: String },
    #[error("I/O error: {message}")]
    Io { code: u32, message: String },
    #[error("serialization error: {message}")]
    Serde { code: u32, message: String },
    #[error("storage error: {message}")]
    Storage { code: u32, message: String },
    #[error("{message}")]
    Other { code: u32, message: String },
}

impl WalletError {
    pub const CODE_MNEMONIC_GENERATION: u32 = 1001;
    pub const CODE_INVALID_MNEMONIC: u32 = 1002;
    pub const CODE_DERIVATION: u32 = 1003;
    pub const CODE_INVALID_ADDRESS: u32 = 1004;
    pub const CODE_TRANSACTION: u32 = 1005;
    pub const CODE_INSUFFICIENT_FUNDS: u32 = 1006;
    pub const CODE_LOCKED_WALLET: u32 = 1007;
    pub const CODE_WRONG_PASSPHRASE: u32 = 1008;
    pub const CODE_RPC: u32 = 2001;
    pub const CODE_ELECTRUM: u32 = 2002;
    pub const CODE_HTTP: u32 = 2003;
    pub const CODE_IO: u32 = 3001;
    pub const CODE_SERDE: u32 = 3002;
    pub const CODE_STORAGE: u32 = 3003;
    pub const CODE_OTHER: u32 = 9999;

    pub fn code(&self) -> u32 {
        match self {
            Self::MnemonicGeneration { code, .. } => *code,
            Self::InvalidMnemonic { code, .. } => *code,
            Self::Derivation { code, .. } => *code,
            Self::InvalidAddress { code, .. } => *code,
            Self::Transaction { code, .. } => *code,
            Self::InsufficientFunds { code, .. } => *code,
            Self::LockedWallet { code } => *code,
            Self::WrongPassphrase { code } => *code,
            Self::Rpc { code, .. } => *code,
            Self::Electrum { code, .. } => *code,
            Self::Http { code, .. } => *code,
            Self::Io { code, .. } => *code,
            Self::Serde { code, .. } => *code,
            Self::Storage { code, .. } => *code,
            Self::Other { code, .. } => *code,
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            code: Self::CODE_OTHER,
            message: message.into(),
        }
    }

    pub fn transaction(message: impl Into<String>) -> Self {
        Self::Transaction {
            code: Self::CODE_TRANSACTION,
            message: message.into(),
        }
    }

    pub fn insufficient_funds(message: impl Into<String>) -> Self {
        Self::InsufficientFunds {
            code: Self::CODE_INSUFFICIENT_FUNDS,
            message: message.into(),
        }
    }

    pub fn is_electrum_rate_limited(&self) -> bool {
        let msg = self.to_string().to_ascii_lowercase();
        msg.contains("too many requests")
            || msg.contains("rate limit")
            || msg.contains("429")
    }

    pub fn is_indexing_budget_exhausted(&self) -> bool {
        self.to_string().contains("indexing budget exhausted")
    }

    pub fn is_electrum_transport(&self) -> bool {
        let msg = self.to_string();
        msg.contains("electrum connect")
            || msg.contains("electrum connection closed")
            || msg.contains("electrum read:")
            || msg.contains("electrum write:")
            || msg.contains("electrum TLS")
            || msg.contains("electrum call timed out")
    }
}

pub type Result<T> = std::result::Result<T, WalletError>;
