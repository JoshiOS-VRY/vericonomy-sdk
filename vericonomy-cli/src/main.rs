//! Vericonomy wallet SDK command-line utilities.

use clap::{Parser, Subcommand};
use vericonomy_chain_params::CoinId;
use vericonomy_hd::{derive_address_at, normalize_mnemonic_phrase};
use vericonomy_wallet_core::validate_mnemonic;

#[derive(Parser)]
#[command(name = "vericonomy-cli", about = "Vericonomy wallet SDK utilities")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate a BIP39 mnemonic (checksum included).
    MnemonicValidate {
        /// Space-separated recovery phrase.
        phrase: String,
    },
    /// Derive a P2PKH receive address at an HD index.
    AddressDerive {
        /// Chain: verium | vericoin
        #[arg(long, default_value = "verium")]
        coin: String,
        /// BIP39 mnemonic or full-node xprv.
        #[arg(long)]
        seed: String,
        /// Optional BIP39 passphrase.
        #[arg(long)]
        passphrase: Option<String>,
        /// Address index (default 0).
        #[arg(long, default_value_t = 0)]
        index: u32,
    },
    /// Emit coin-profiles.json for TypeScript shells.
    ProfilesJson,
}

fn parse_coin(s: &str) -> Result<CoinId, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "verium" | "vrm" => Ok(CoinId::Verium),
        "vericoin" | "vrc" => Ok(CoinId::Vericoin),
        other => Err(format!("unknown coin: {other}")),
    }
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Commands::MnemonicValidate { phrase } => {
            let normalized = normalize_mnemonic_phrase(&phrase);
            if validate_mnemonic(&normalized) {
                println!("valid");
                0
            } else {
                eprintln!("invalid mnemonic");
                1
            }
        }
        Commands::AddressDerive {
            coin,
            seed,
            passphrase,
            index,
        } => match parse_coin(&coin) {
            Ok(coin_id) => match derive_address_at(coin_id, &seed, passphrase.as_deref(), index) {
                Ok(addr) => {
                    println!("{addr}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            },
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Commands::ProfilesJson => match vericonomy_chain_params::profiles_json_string() {
            Ok(json) => {
                print!("{json}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
    };
    std::process::exit(code);
}
