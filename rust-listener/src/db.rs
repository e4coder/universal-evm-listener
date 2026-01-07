use crate::types::Transfer;
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Lock error")]
    Lock,
}

/// SQLite database wrapper with WAL mode for concurrent access
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open database and create tables if needed
    pub fn open(path: &str) -> Result<Self, DbError> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(path)?;

        // Enable WAL mode for concurrent reads while writing
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = 10000;
             PRAGMA temp_store = MEMORY;"
        )?;

        // Create transfers table with deduplication via UNIQUE constraint
        conn.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chain_id INTEGER NOT NULL,
                tx_hash TEXT NOT NULL,
                log_index INTEGER NOT NULL,
                token TEXT NOT NULL,
                from_addr TEXT NOT NULL,
                to_addr TEXT NOT NULL,
                value TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                block_timestamp INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            [],
        )?;

        // Create indexes for query patterns
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_from ON transfers(chain_id, from_addr, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_to ON transfers(chain_id, to_addr, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_created ON transfers(created_at)",
            [],
        )?;

        // Create checkpoints table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                chain_id INTEGER PRIMARY KEY,
                block_number INTEGER NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert a transfer, ignoring duplicates (via UNIQUE constraint)
    pub fn insert_transfer(&self, transfer: &Transfer) -> Result<bool, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "INSERT OR IGNORE INTO transfers
             (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                transfer.chain_id,
                transfer.tx_hash,
                transfer.log_index,
                transfer.token.to_lowercase(),
                transfer.from_addr.to_lowercase(),
                transfer.to_addr.to_lowercase(),
                transfer.value,
                transfer.block_number,
                transfer.block_timestamp,
                now
            ],
        )?;

        // Returns true if a row was inserted (not a duplicate)
        Ok(result > 0)
    }

    /// Insert multiple transfers in a batch transaction
    pub fn insert_transfers_batch(&self, transfers: &[Transfer]) -> Result<usize, DbError> {
        let mut conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let tx = conn.transaction()?;
        let mut inserted = 0;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO transfers
                 (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            )?;

            for transfer in transfers {
                let result = stmt.execute(params![
                    transfer.chain_id,
                    transfer.tx_hash,
                    transfer.log_index,
                    transfer.token.to_lowercase(),
                    transfer.from_addr.to_lowercase(),
                    transfer.to_addr.to_lowercase(),
                    transfer.value,
                    transfer.block_number,
                    transfer.block_timestamp,
                    now
                ])?;
                if result > 0 {
                    inserted += 1;
                }
            }
        }

        tx.commit()?;
        Ok(inserted)
    }

    /// Get checkpoint for a chain
    pub fn get_checkpoint(&self, chain_id: u32) -> Result<Option<u64>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let result = conn.query_row(
            "SELECT block_number FROM checkpoints WHERE chain_id = ?1",
            params![chain_id],
            |row| row.get(0),
        );

        match result {
            Ok(block) => Ok(Some(block)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Set checkpoint for a chain
    pub fn set_checkpoint(&self, chain_id: u32, block_number: u64) -> Result<(), DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        conn.execute(
            "INSERT INTO checkpoints (chain_id, block_number, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(chain_id) DO UPDATE SET
             block_number = excluded.block_number,
             updated_at = excluded.updated_at",
            params![chain_id, block_number, now],
        )?;

        Ok(())
    }

    /// Clean up old transfers based on TTL
    pub fn cleanup_old(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - ttl_secs;

        let deleted = conn.execute(
            "DELETE FROM transfers WHERE created_at < ?1",
            params![cutoff],
        )?;

        Ok(deleted)
    }

    /// Get total count of transfers (for monitoring)
    pub fn get_transfer_count(&self) -> Result<u64, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM transfers",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get transfers by 'from' address
    pub fn get_transfers_by_from(
        &self,
        chain_id: u32,
        from: &str,
        limit: u32,
    ) -> Result<Vec<Transfer>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE chain_id = ?1 AND from_addr = ?2
             ORDER BY block_timestamp DESC
             LIMIT ?3"
        )?;

        let transfers = stmt
            .query_map(params![chain_id, from.to_lowercase(), limit], |row| {
                Ok(Transfer {
                    chain_id: row.get(0)?,
                    tx_hash: row.get(1)?,
                    log_index: row.get(2)?,
                    token: row.get(3)?,
                    from_addr: row.get(4)?,
                    to_addr: row.get(5)?,
                    value: row.get(6)?,
                    block_number: row.get(7)?,
                    block_timestamp: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    /// Get transfers by 'to' address
    pub fn get_transfers_by_to(
        &self,
        chain_id: u32,
        to: &str,
        limit: u32,
    ) -> Result<Vec<Transfer>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE chain_id = ?1 AND to_addr = ?2
             ORDER BY block_timestamp DESC
             LIMIT ?3"
        )?;

        let transfers = stmt
            .query_map(params![chain_id, to.to_lowercase(), limit], |row| {
                Ok(Transfer {
                    chain_id: row.get(0)?,
                    tx_hash: row.get(1)?,
                    log_index: row.get(2)?,
                    token: row.get(3)?,
                    from_addr: row.get(4)?,
                    to_addr: row.get(5)?,
                    value: row.get(6)?,
                    block_number: row.get(7)?,
                    block_timestamp: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }
}
