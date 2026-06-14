//! Electrum JSON-RPC line protocol (v1.4).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct ElectrumRequest {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Deserialize)]
pub struct ElectrumResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<ElectrumError>,
    pub method: Option<String>,
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ElectrumError {
    pub code: i32,
    pub message: String,
}

impl ElectrumError {
    pub fn is_rate_limited(&self) -> bool {
        self.code == -101
    }
}

use vericonomy_errors::{Result, WalletError};

impl ElectrumResponse {
    pub fn into_result(self) -> Result<Value> {
        if let Some(err) = self.error {
            return Err(WalletError::Electrum {
                code: err.code.unsigned_abs(),
                message: err.message,
            });
        }
        self.result
            .ok_or_else(|| WalletError::other("electrum response missing result"))
    }

    pub fn is_notification(&self) -> bool {
        self.id.is_none() && self.method.is_some()
    }
}
