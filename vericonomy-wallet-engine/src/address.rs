//! P2PKH send-address format checks shared by light and full-node sends.

use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_hd;

pub const P2PKH_ADDRESS_LEN: usize = 34;

const BASE58_CHARS: &str =
    "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Validate a destination address before building or broadcasting a send.
pub fn validate_send_address(coin: CoinId, address: &str) -> Result<()> {
    let address = address.trim();
    if address.is_empty() {
        return Err(WalletError::other("address is required"));
    }
    if !address.starts_with('V') {
        return Err(WalletError::other("address must start with V"));
    }
    if address.len() != P2PKH_ADDRESS_LEN {
        return Err(WalletError::other(format!(
            "address must be exactly {P2PKH_ADDRESS_LEN} characters"
        )));
    }
    if !address.chars().all(|c| BASE58_CHARS.contains(c)) {
        return Err(WalletError::other(
            "address must contain only letters and numbers (no spaces or symbols)",
        ));
    }
    vericonomy_hd::address_to_script_pubkey(coin, address)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_verium_address() {
        validate_send_address(
            CoinId::Verium,
            "VRq98Nm2P6anLHPgnHdb6NnibJ6GoG3Jm9",
        )
        .unwrap();
    }

    #[test]
    fn rejects_wrong_prefix() {
        let err = validate_send_address(CoinId::Verium, "BY6E3KSqrMk1hcy5Cu4EGyHrdDS5ch3YHU")
            .unwrap_err()
            .to_string();
        assert!(err.contains("start with V"));
    }

    #[test]
    fn rejects_wrong_length() {
        let err = validate_send_address(CoinId::Verium, "VY6E3KSqrMk1hcy5Cu4EGyHrdDS5ch3YH")
            .unwrap_err()
            .to_string();
        assert!(err.contains("34 characters"));
    }

    #[test]
    fn rejects_invalid_checksum() {
        let err = validate_send_address(
            CoinId::Verium,
            "VRq98Nm2P6anLHPgnHdb6NnibJ6GoG3Jm!",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("invalid address") || err.contains("letters and numbers"));
    }

    #[test]
    fn rejects_special_characters() {
        let err = validate_send_address(
            CoinId::Verium,
            "VRq98Nm2P6anLHPgnHdb6NnibJ6GoG3Jm!",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("letters and numbers"));
    }
}
