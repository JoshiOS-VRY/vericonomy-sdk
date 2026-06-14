//! Process-wide Electrum rate-limit backoff (per coin).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use vericonomy_chain_params::CoinId;

/// Pause Electrum RPC after the server reports excessive usage (-101).
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(300);
/// Minimum gap between `blockchain.headers.subscribe` status probes.
pub const STATUS_PROBE_INTERVAL: Duration = Duration::from_secs(10);

static COOLDOWN_UNTIL: Lazy<Mutex<HashMap<CoinId, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static LAST_STATUS_PROBE: Lazy<Mutex<HashMap<CoinId, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn record_rate_limit(coin: CoinId) {
    let until = Instant::now() + RATE_LIMIT_COOLDOWN;
    if let Ok(mut map) = COOLDOWN_UNTIL.lock() {
        map.insert(coin, until);
    }
    tracing::warn!(
        "electrum rate limit for {} — pausing RPC for {}s",
        coin.as_str(),
        RATE_LIMIT_COOLDOWN.as_secs()
    );
}

pub fn is_in_cooldown(coin: CoinId) -> bool {
    match COOLDOWN_UNTIL.lock() {
        Ok(map) => map
            .get(&coin)
            .map(|until| Instant::now() < *until)
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub fn clear_cooldown(coin: CoinId) {
    if let Ok(mut map) = COOLDOWN_UNTIL.lock() {
        map.remove(&coin);
    }
}

pub fn status_probe_due(coin: CoinId) -> bool {
    let now = Instant::now();
    match LAST_STATUS_PROBE.lock() {
        Ok(map) => match map.get(&coin) {
            Some(last) => now.duration_since(*last) >= STATUS_PROBE_INTERVAL,
            None => true,
        },
        Err(_) => true,
    }
}

pub fn mark_status_probe(coin: CoinId) {
    if let Ok(mut map) = LAST_STATUS_PROBE.lock() {
        map.insert(coin, Instant::now());
    }
}
