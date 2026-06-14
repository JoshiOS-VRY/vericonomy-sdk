//! Golden vectors for Vericonomy transaction wire format (version + nTime extension).

use bitcoin::absolute::LockTime;
use bitcoin::transaction::Version;
use bitcoin::{OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness};
use vericonomy_tx::{
    decode_verium_tx, display_txid_from_raw, serialize_verium_tx, wire_txid_from_raw,
    VeriumMutableTx,
};

fn golden_empty_tx() -> VeriumMutableTx {
    VeriumMutableTx {
        version: Version::ONE.0,
        n_time: 42,
        inputs: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        outputs: vec![TxOut {
            value: bitcoin::Amount::from_sat(0),
            script_pubkey: ScriptBuf::from_bytes(
                hex::decode("76a914000000000000000000000000000000000000000088ac").unwrap(),
            ),
        }],
        lock_time: LockTime::ZERO.to_consensus_u32(),
    }
}

#[test]
fn golden_serialize_places_n_time_after_version() {
    let tx = golden_empty_tx();
    let raw = serialize_verium_tx(&tx).unwrap();
    assert_eq!(&raw[0..4], &[1, 0, 0, 0], "version LE");
    assert_eq!(
        u32::from_le_bytes(raw[4..8].try_into().unwrap()),
        42,
        "nTime LE"
    );
    assert_eq!(raw[8], 1, "vin count");
}

#[test]
fn golden_round_trip_matches_struct() {
    let tx = golden_empty_tx();
    let raw = serialize_verium_tx(&tx).unwrap();
    let decoded = decode_verium_tx(&raw).unwrap();
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.n_time, 42);
    assert_eq!(decoded.inputs.len(), 1);
    assert_eq!(decoded.outputs.len(), 1);
    assert_eq!(decoded.lock_time, 0);
}

#[test]
fn golden_display_txid_is_stable() {
    let tx = golden_empty_tx();
    let raw = serialize_verium_tx(&tx).unwrap();
    let display = display_txid_from_raw(&raw);
    let wire = wire_txid_from_raw(&raw);
    assert_eq!(display.parse::<bitcoin::Txid>().unwrap(), wire);
    assert_eq!(
        display,
        "0a7c4a053f45ba256976b12ffedce9ceba1f35634dd3099c94c47f4a1248283e"
    );
}

#[test]
fn golden_hex_prefix_documents_n_time_field() {
    let tx = golden_empty_tx();
    let raw = serialize_verium_tx(&tx).unwrap();
    let hex = hex::encode(&raw);
    assert!(hex.starts_with("01000000"), "version");
    assert!(hex[8..16].eq_ignore_ascii_case("2a000000"), "nTime=42");
}
