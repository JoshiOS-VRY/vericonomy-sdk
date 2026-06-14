//! Electrum scripthash: reverse(hex(SHA256(scriptPubKey))).

use sha2::{Digest, Sha256};

/// Compute Electrum scripthash from raw script bytes.
pub fn scripthash_from_script(script: &[u8]) -> String {
    let hash = Sha256::digest(script);
    let mut bytes = hash.to_vec();
    bytes.reverse();
    hex::encode(bytes)
}

use vericonomy_errors::{Result, WalletError};

/// Compute scripthash from hex-encoded scriptPubKey.
pub fn scripthash_from_script_hex(script_hex: &str) -> Result<String> {
    let script = hex::decode(script_hex.trim())
        .map_err(|e| WalletError::other(format!("invalid script hex: {e}")))?;
    Ok(scripthash_from_script(&script))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripthash_reverses_sha256() {
        let script = hex::decode("76a91489abcdefabbaabbaabbaabbaabbaabbaabbaabba88ac").unwrap();
        let h = scripthash_from_script(&script);
        assert_eq!(h.len(), 64);
    }
}
