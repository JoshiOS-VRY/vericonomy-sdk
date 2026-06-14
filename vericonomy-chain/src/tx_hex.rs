//! Normalize and parse raw transaction hex from RPC / Electrum responses.

use serde_json::Value;

use vericonomy_errors::{Result, WalletError};

/// Strip optional `0x`, whitespace, and non-hex characters; require even length.
pub fn normalize_transaction_hex(raw: &str, context: &str) -> Result<String> {
    let trimmed = raw.trim();
    let without_prefix = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let cleaned: String = without_prefix
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if cleaned.is_empty() {
        return Err(WalletError::other(format!("{context}: hex is empty")));
    }
    if cleaned.len() % 2 != 0 {
        return Err(WalletError::other(format!(
            "{context}: odd number of digits ({})",
            cleaned.len()
        )));
    }
    Ok(cleaned.to_ascii_lowercase())
}

/// Electrum `blockchain.transaction.get` may return a raw hex string (`verbose=false`)
/// or a decoded object with a `hex` field (`verbose=true`).
pub fn parse_electrum_transaction_get(value: &Value) -> Result<String> {
    if let Some(s) = value.as_str() {
        return normalize_transaction_hex(s, "transaction hex");
    }
    if let Some(hex) = value.get("hex").and_then(Value::as_str) {
        return normalize_transaction_hex(hex, "transaction hex");
    }
    Err(WalletError::other(
        "electrum transaction.get: expected hex string or object with hex field",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_strips_prefix_and_whitespace() {
        let out = normalize_transaction_hex(" 0xAB cd ", "test").unwrap();
        assert_eq!(out, "abcd");
    }

    #[test]
    fn normalize_rejects_odd_length() {
        assert!(normalize_transaction_hex("abc", "test").is_err());
    }

    #[test]
    fn parse_verbose_false_string() {
        let hex = parse_electrum_transaction_get(&json!("01020304")).unwrap();
        assert_eq!(hex, "01020304");
    }

    #[test]
    fn parse_verbose_true_object() {
        let hex = parse_electrum_transaction_get(&json!({
            "txid": "deadbeef",
            "hex": "01020304"
        }))
        .unwrap();
        assert_eq!(hex, "01020304");
    }
}
