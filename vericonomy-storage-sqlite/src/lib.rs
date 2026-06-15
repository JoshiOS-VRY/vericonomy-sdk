//! SQLite-backed UTXO and transaction history cache.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use vericonomy_chain::history_rows::wallet_tx_row_key;
use vericonomy_chain::types::{Utxo, WalletTx};
use vericonomy_chain_params::CoinId;
use vericonomy_errors::{Result, WalletError};
use vericonomy_storage::{TxCache, UtxoCache};

pub struct SqliteWalletCache {
    base_dir: PathBuf,
}

impl Clone for SqliteWalletCache {
    fn clone(&self) -> Self {
        Self {
            base_dir: self.base_dir.clone(),
        }
    }
}

impl SqliteWalletCache {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
        }
    }

    fn open(&self, coin: CoinId) -> Result<Connection> {
        let path = self.cache_path(coin);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WalletError::Io {
                code: WalletError::CODE_IO,
                message: e.to_string(),
            })?;
        }
        let conn = Connection::open(&path).map_err(|e| WalletError::other(format!("sqlite open: {e}")))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS utxo_cache (
                txid TEXT NOT NULL,
                vout INTEGER NOT NULL,
                value_sats INTEGER NOT NULL,
                script_hex TEXT NOT NULL,
                height INTEGER NOT NULL,
                fetched_at INTEGER NOT NULL,
                PRIMARY KEY (txid, vout)
            );
            CREATE TABLE IF NOT EXISTS tx_history (
                txid TEXT NOT NULL,
                category TEXT NOT NULL,
                sort_time INTEGER NOT NULL,
                payload TEXT NOT NULL,
                PRIMARY KEY (txid, category)
            );",
        )
        .map_err(|e| WalletError::other(format!("sqlite schema: {e}")))?;
        Ok(conn)
    }

    fn cache_path(&self, coin: CoinId) -> PathBuf {
        self.base_dir
            .join(format!("light-cache-{}.sqlite", coin.as_str()))
    }

    fn set_meta_sync(&self, coin: CoinId, key: &str, value: &str) -> Result<()> {
        let conn = self.open(coin)?;
        conn.execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| WalletError::other(format!("sqlite meta: {e}")))?;
        Ok(())
    }

    fn get_meta_sync(&self, coin: CoinId, key: &str) -> Result<Option<String>> {
        let conn = self.open(coin)?;
        let mut stmt = conn
            .prepare("SELECT value FROM sync_meta WHERE key = ?1")
            .map_err(|e| WalletError::other(format!("sqlite prepare: {e}")))?;
        let mut rows = stmt
            .query(params![key])
            .map_err(|e| WalletError::other(format!("sqlite query: {e}")))?;
        if let Some(row) = rows.next().map_err(|e| WalletError::other(format!("sqlite row: {e}")))? {
            let v: String = row.get(0).map_err(|e| WalletError::other(format!("sqlite get: {e}")))?;
            return Ok(Some(v));
        }
        Ok(None)
    }

    fn list_utxos_sync(&self, coin: CoinId) -> Result<Vec<Utxo>> {
        let conn = self.open(coin)?;
        let mut stmt = conn
            .prepare(
                "SELECT txid, vout, value_sats, script_hex, height FROM utxo_cache ORDER BY height DESC",
            )
            .map_err(|e| WalletError::other(format!("sqlite prepare: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Utxo {
                    txid: row.get(0)?,
                    vout: row.get(1)?,
                    value_sats: row.get(2)?,
                    script_hex: row.get(3)?,
                    height: row.get(4)?,
                    address: String::new(),
                    confirmations: 0,
                })
            })
            .map_err(|e| WalletError::other(format!("sqlite query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| WalletError::other(format!("sqlite row: {e}")))?);
        }
        Ok(out)
    }

    fn replace_utxos_sync(&self, coin: CoinId, utxos: &[Utxo]) -> Result<bool> {
        let conn = self.open(coin)?;
        let mut changed = false;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut existing: HashMap<(String, u32), (i64, String, u32)> = HashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT txid, vout, value_sats, script_hex, height FROM utxo_cache")
                .map_err(|e| WalletError::other(format!("sqlite prepare: {e}")))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, u32>(1)?),
                        (
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, u32>(4)?,
                        ),
                    ))
                })
                .map_err(|e| WalletError::other(format!("sqlite query: {e}")))?;
            for row in rows {
                let (k, v) = row.map_err(|e| WalletError::other(format!("sqlite row: {e}")))?;
                existing.insert(k, v);
            }
        }

        let incoming_keys: HashSet<(String, u32)> =
            utxos.iter().map(|u| (u.txid.clone(), u.vout)).collect();

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| WalletError::other(format!("sqlite tx: {e}")))?;

        {
            let mut upsert = tx
                .prepare(
                    "INSERT INTO utxo_cache(txid, vout, value_sats, script_hex, height, fetched_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(txid, vout) DO UPDATE SET
                       value_sats = excluded.value_sats,
                       script_hex = excluded.script_hex,
                       height = excluded.height,
                       fetched_at = excluded.fetched_at",
                )
                .map_err(|e| WalletError::other(format!("sqlite prepare upsert: {e}")))?;
            for u in utxos {
                let unchanged = matches!(
                    existing.get(&(u.txid.clone(), u.vout)),
                    Some((value, script, height))
                        if *value == u.value_sats && script == &u.script_hex && *height == u.height
                );
                if unchanged {
                    continue;
                }
                changed = true;
                upsert
                    .execute(params![
                        u.txid,
                        u.vout,
                        u.value_sats,
                        u.script_hex,
                        u.height,
                        now
                    ])
                    .map_err(|e| WalletError::other(format!("sqlite utxo upsert: {e}")))?;
            }
        }

        {
            let mut delete = tx
                .prepare("DELETE FROM utxo_cache WHERE txid = ?1 AND vout = ?2")
                .map_err(|e| WalletError::other(format!("sqlite prepare delete: {e}")))?;
            for (txid, vout) in existing.keys() {
                if !incoming_keys.contains(&(txid.clone(), *vout)) {
                    changed = true;
                    delete
                        .execute(params![txid, vout])
                        .map_err(|e| WalletError::other(format!("sqlite utxo delete: {e}")))?;
                }
            }
        }

        tx.commit()
            .map_err(|e| WalletError::other(format!("sqlite commit: {e}")))?;
        Ok(changed)
    }

    fn replace_tx_history_sync(&self, coin: CoinId, txs: &[WalletTx]) -> Result<()> {
        let conn = self.open(coin)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| WalletError::other(format!("sqlite tx: {e}")))?;
        tx.execute("DELETE FROM tx_history", [])
            .map_err(|e| WalletError::other(format!("sqlite clear history: {e}")))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO tx_history(txid, category, sort_time, payload)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| WalletError::other(format!("sqlite prepare history: {e}")))?;
            for t in txs {
                let payload = serde_json::to_string(t)
                    .map_err(|e| WalletError::Serde {
                        code: WalletError::CODE_SERDE,
                        message: e.to_string(),
                    })?;
                let sort_time = t.time.unwrap_or(0) as i64;
                // `category` column stores the full row key (txid:category:address), matching Tauri.
                let storage_category = wallet_tx_row_key(t);
                stmt.execute(params![t.txid, storage_category, sort_time, payload])
                    .map_err(|e| WalletError::other(format!("sqlite history insert: {e}")))?;
            }
        }
        tx.commit()
            .map_err(|e| WalletError::other(format!("sqlite commit: {e}")))?;
        Ok(())
    }

    fn list_tx_history_sync(&self, coin: CoinId, limit: usize) -> Result<Vec<WalletTx>> {
        let conn = self.open(coin)?;
        let mut stmt = conn
            .prepare("SELECT payload FROM tx_history ORDER BY sort_time DESC LIMIT ?1")
            .map_err(|e| WalletError::other(format!("sqlite prepare: {e}")))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .map_err(|e| WalletError::other(format!("sqlite query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let payload = row.map_err(|e| WalletError::other(format!("sqlite row: {e}")))?;
            if let Ok(tx) = serde_json::from_str::<WalletTx>(&payload) {
                out.push(tx);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl UtxoCache for SqliteWalletCache {
    async fn list_utxos(&self, coin: CoinId) -> Result<Vec<Utxo>> {
        self.list_utxos_sync(coin)
    }

    async fn replace_utxos(&self, coin: CoinId, utxos: &[Utxo]) -> Result<bool> {
        self.replace_utxos_sync(coin, utxos)
    }

    async fn upsert_utxo(&self, coin: CoinId, utxo: &Utxo) -> Result<()> {
        let mut utxos = self.list_utxos_sync(coin)?;
        if let Some(existing) = utxos
            .iter_mut()
            .find(|u| u.txid == utxo.txid && u.vout == utxo.vout)
        {
            *existing = utxo.clone();
        } else {
            utxos.push(utxo.clone());
        }
        self.replace_utxos_sync(coin, &utxos)?;
        Ok(())
    }

    async fn remove_utxo(&self, coin: CoinId, txid: &str, vout: u32) -> Result<bool> {
        let conn = self.open(coin)?;
        let n = conn
            .execute(
                "DELETE FROM utxo_cache WHERE txid = ?1 AND vout = ?2",
                params![txid, vout],
            )
            .map_err(|e| WalletError::other(format!("sqlite utxo delete: {e}")))?;
        Ok(n > 0)
    }

    async fn get_meta(&self, coin: CoinId, key: &str) -> Result<Option<String>> {
        self.get_meta_sync(coin, key)
    }

    async fn set_meta(&self, coin: CoinId, key: &str, value: &str) -> Result<()> {
        self.set_meta_sync(coin, key, value)
    }

    async fn clear_coin(&self, coin: CoinId) -> Result<()> {
        let path = self.cache_path(coin);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| WalletError::Io {
                code: WalletError::CODE_IO,
                message: e.to_string(),
            })?;
        }
        Ok(())
    }
}

#[async_trait]
impl TxCache for SqliteWalletCache {
    async fn load_history(&self, coin: CoinId, limit: usize) -> Result<Vec<WalletTx>> {
        self.list_tx_history_sync(coin, limit)
    }

    async fn replace_history(&self, coin: CoinId, txs: &[WalletTx]) -> Result<()> {
        self.replace_tx_history_sync(coin, txs)
    }

    async fn clear_history(&self, coin: CoinId) -> Result<()> {
        let conn = self.open(coin)?;
        conn.execute("DELETE FROM tx_history", [])
            .map_err(|e| WalletError::other(format!("sqlite clear history: {e}")))?;
        Ok(())
    }
}
