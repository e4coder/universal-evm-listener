use crate::types::{Crypto2FiatEvent, DstEscrowCreatedData, FusionPlusSwap, FusionSwap, Transfer};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("Chain database not found: {0}")]
    ChainNotFound(u32),
}

// =============================================================================
// ChainDatabase - Per-chain SQLite database with connection pool
// =============================================================================

/// Per-chain SQLite database for transfers and checkpoints
/// Each chain gets its own database file for true write parallelism
pub struct ChainDatabase {
    pool: Pool<SqliteConnectionManager>,
    chain_id: u32,
}

impl ChainDatabase {
    /// Open or create a chain-specific database
    pub fn open(data_dir: &str, chain_id: u32) -> Result<Self, DbError> {
        let path = format!("{}/chain_{}.db", data_dir, chain_id);

        // Create parent directory if needed
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let manager = SqliteConnectionManager::file(&path);

        let pool = Pool::builder()
            .max_size(3) // Each chain only needs a few connections
            .min_idle(Some(1))
            .connection_timeout(Duration::from_secs(10))
            .build(manager)?;

        // Initialize connection with pragmas and create tables
        {
            let conn = pool.get()?;
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA cache_size = 2000;
                 PRAGMA temp_store = MEMORY;
                 PRAGMA wal_autocheckpoint = 1000;
                 PRAGMA busy_timeout = 5000;"
            )?;

            Self::create_chain_tables(&conn)?;
        }

        Ok(Self { pool, chain_id })
    }

    fn create_chain_tables(conn: &rusqlite::Connection) -> Result<(), DbError> {
        // transfers table (chain_id column not needed since it's implicit from file)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_hash TEXT NOT NULL,
                log_index INTEGER NOT NULL,
                token TEXT NOT NULL,
                from_addr TEXT NOT NULL,
                to_addr TEXT NOT NULL,
                value TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                block_timestamp INTEGER NOT NULL,
                swap_type TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                UNIQUE(tx_hash, log_index)
            )",
            [],
        )?;

        // checkpoint table (single row per database)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoint (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                block_number INTEGER NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create indexes for query patterns
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_from ON transfers(from_addr, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_to ON transfers(to_addr, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tx_hash ON transfers(tx_hash)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_created ON transfers(created_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_swap_type ON transfers(swap_type, block_timestamp DESC)",
            [],
        )?;

        // Indexes for efficient since_id streaming queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_from_id ON transfers(from_addr, id ASC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_to_id ON transfers(to_addr, id ASC)",
            [],
        )?;

        Ok(())
    }

    pub fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>, DbError> {
        Ok(self.pool.get()?)
    }

    pub fn chain_id(&self) -> u32 {
        self.chain_id
    }

    /// Insert a transfer, ignoring duplicates
    pub fn insert_transfer(&self, transfer: &Transfer) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "INSERT OR IGNORE INTO transfers
             (tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
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

        Ok(result > 0)
    }

    /// Insert multiple transfers in a batch transaction
    pub fn insert_transfers_batch(&self, transfers: &[Transfer]) -> Result<usize, DbError> {
        let mut conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let tx = conn.transaction()?;
        let mut inserted = 0;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO transfers
                 (tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            )?;

            for transfer in transfers {
                let result = stmt.execute(params![
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

    /// Get checkpoint block number
    pub fn get_checkpoint(&self) -> Result<Option<u64>, DbError> {
        let conn = self.get_conn()?;
        let result = conn.query_row(
            "SELECT block_number FROM checkpoint WHERE id = 1",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(block) => Ok(Some(block)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Set checkpoint block number
    pub fn set_checkpoint(&self, block_number: u64) -> Result<(), DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        conn.execute(
            "INSERT INTO checkpoint (id, block_number, updated_at)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
             block_number = excluded.block_number,
             updated_at = excluded.updated_at",
            params![block_number, now],
        )?;

        Ok(())
    }

    /// Clean up old transfers based on TTL
    pub fn cleanup_old(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.get_conn()?;
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

    /// Force a WAL checkpoint
    pub fn checkpoint(&self) -> Result<(), DbError> {
        let conn = self.get_conn()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Get total count of transfers
    pub fn get_transfer_count(&self) -> Result<u64, DbError> {
        let conn = self.get_conn()?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM transfers",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get transfers by 'from' address
    pub fn get_transfers_by_from(&self, from: &str, limit: u32) -> Result<Vec<Transfer>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE from_addr = ?1
             ORDER BY block_timestamp DESC
             LIMIT ?2"
        )?;

        let chain_id = self.chain_id;
        let transfers = stmt
            .query_map(params![from.to_lowercase(), limit], |row| {
                Ok(Transfer {
                    chain_id,
                    tx_hash: row.get(0)?,
                    log_index: row.get(1)?,
                    token: row.get(2)?,
                    from_addr: row.get(3)?,
                    to_addr: row.get(4)?,
                    value: row.get(5)?,
                    block_number: row.get(6)?,
                    block_timestamp: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    /// Get transfers by 'to' address
    pub fn get_transfers_by_to(&self, to: &str, limit: u32) -> Result<Vec<Transfer>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE to_addr = ?1
             ORDER BY block_timestamp DESC
             LIMIT ?2"
        )?;

        let chain_id = self.chain_id;
        let transfers = stmt
            .query_map(params![to.to_lowercase(), limit], |row| {
                Ok(Transfer {
                    chain_id,
                    tx_hash: row.get(0)?,
                    log_index: row.get(1)?,
                    token: row.get(2)?,
                    from_addr: row.get(3)?,
                    to_addr: row.get(4)?,
                    value: row.get(5)?,
                    block_number: row.get(6)?,
                    block_timestamp: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    /// Get transfers by transaction hash
    pub fn get_transfers_by_tx_hash(&self, tx_hash: &str) -> Result<Vec<Transfer>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE tx_hash = ?1
             ORDER BY log_index ASC"
        )?;

        let chain_id = self.chain_id;
        let transfers = stmt
            .query_map(params![tx_hash.to_lowercase()], |row| {
                Ok(Transfer {
                    chain_id,
                    tx_hash: row.get(0)?,
                    log_index: row.get(1)?,
                    token: row.get(2)?,
                    from_addr: row.get(3)?,
                    to_addr: row.get(4)?,
                    value: row.get(5)?,
                    block_number: row.get(6)?,
                    block_timestamp: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    /// Label transfers in a transaction with swap_type
    pub fn label_transfers_as_fusion(&self, tx_hash: &str, swap_type: &str) -> Result<usize, DbError> {
        let conn = self.get_conn()?;

        let result = conn.execute(
            "UPDATE transfers SET swap_type = ?1 WHERE tx_hash = ?2",
            params![swap_type, tx_hash.to_lowercase()],
        )?;

        Ok(result)
    }
}

// =============================================================================
// SharedDatabase - Shared database for cross-chain data
// =============================================================================

/// Shared database for cross-chain data (Fusion+, Fusion swaps, Crypto2Fiat)
pub struct SharedDatabase {
    pool: Pool<SqliteConnectionManager>,
}

impl SharedDatabase {
    /// Open or create the shared database
    pub fn open(data_dir: &str) -> Result<Self, DbError> {
        let path = format!("{}/shared.db", data_dir);

        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let manager = SqliteConnectionManager::file(&path);

        let pool = Pool::builder()
            .max_size(15) // All chains may need to write/read fusion data
            .min_idle(Some(3))
            .connection_timeout(Duration::from_secs(30))
            .build(manager)?;

        // Initialize connection with pragmas and create tables
        {
            let conn = pool.get()?;
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA cache_size = 2000;
                 PRAGMA temp_store = MEMORY;
                 PRAGMA wal_autocheckpoint = 1000;
                 PRAGMA busy_timeout = 30000;"
            )?;

            Self::create_shared_tables(&conn)?;
        }

        Ok(Self { pool })
    }

    fn create_shared_tables(conn: &rusqlite::Connection) -> Result<(), DbError> {
        // fusion_plus_swaps table for 1inch Fusion+ cross-chain swaps
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fusion_plus_swaps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                order_hash TEXT NOT NULL UNIQUE,
                hashlock TEXT NOT NULL,
                secret TEXT,
                src_chain_id INTEGER NOT NULL,
                src_tx_hash TEXT NOT NULL,
                src_block_number INTEGER NOT NULL,
                src_block_timestamp INTEGER NOT NULL,
                src_log_index INTEGER NOT NULL,
                src_escrow_address TEXT,
                src_maker TEXT NOT NULL,
                src_taker TEXT NOT NULL,
                src_token TEXT NOT NULL,
                src_amount TEXT NOT NULL,
                src_safety_deposit TEXT NOT NULL,
                src_timelocks TEXT NOT NULL,
                src_status TEXT NOT NULL DEFAULT 'created',
                dst_chain_id INTEGER NOT NULL,
                dst_tx_hash TEXT,
                dst_block_number INTEGER,
                dst_block_timestamp INTEGER,
                dst_log_index INTEGER,
                dst_escrow_address TEXT,
                dst_maker TEXT NOT NULL,
                dst_taker TEXT,
                dst_token TEXT NOT NULL,
                dst_amount TEXT NOT NULL,
                dst_safety_deposit TEXT NOT NULL,
                dst_timelocks TEXT,
                dst_status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // fusion_swaps table for 1inch Fusion single-chain swaps
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fusion_swaps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                order_hash TEXT NOT NULL,
                chain_id INTEGER NOT NULL,
                tx_hash TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                block_timestamp INTEGER NOT NULL,
                log_index INTEGER NOT NULL,
                maker TEXT NOT NULL,
                taker TEXT,
                maker_token TEXT,
                taker_token TEXT,
                maker_amount TEXT,
                taker_amount TEXT,
                remaining TEXT NOT NULL,
                is_partial_fill INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'filled',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            [],
        )?;

        // crypto2fiat_events table for KentuckyDelegate crypto-to-fiat offramps
        conn.execute(
            "CREATE TABLE IF NOT EXISTS crypto2fiat_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                order_id TEXT NOT NULL,
                token TEXT NOT NULL,
                amount TEXT NOT NULL,
                recipient TEXT NOT NULL,
                metadata TEXT,
                chain_id INTEGER NOT NULL,
                tx_hash TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                block_timestamp INTEGER NOT NULL,
                log_index INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            [],
        )?;

        // Create indexes for fusion_plus_swaps
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_hashlock ON fusion_plus_swaps(hashlock)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_src_chain ON fusion_plus_swaps(src_chain_id, src_block_timestamp DESC)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_dst_chain ON fusion_plus_swaps(dst_chain_id, dst_block_timestamp DESC)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_src_maker ON fusion_plus_swaps(src_maker)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_dst_maker ON fusion_plus_swaps(dst_maker)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_src_taker ON fusion_plus_swaps(src_taker)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_status ON fusion_plus_swaps(src_status, dst_status)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fp_created ON fusion_plus_swaps(created_at)", [])?;

        // Create indexes for fusion_swaps
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_order_hash ON fusion_swaps(order_hash)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_chain ON fusion_swaps(chain_id, block_timestamp DESC)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_maker ON fusion_swaps(maker)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_taker ON fusion_swaps(taker)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_status ON fusion_swaps(status)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_fs_created ON fusion_swaps(created_at)", [])?;

        // Create indexes for crypto2fiat_events
        conn.execute("CREATE INDEX IF NOT EXISTS idx_c2f_order_id ON crypto2fiat_events(order_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_c2f_token ON crypto2fiat_events(token)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_c2f_recipient ON crypto2fiat_events(recipient)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_c2f_chain ON crypto2fiat_events(chain_id, block_timestamp DESC)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_c2f_created ON crypto2fiat_events(created_at)", [])?;

        Ok(())
    }

    pub fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>, DbError> {
        Ok(self.pool.get()?)
    }

    /// Force a WAL checkpoint
    pub fn checkpoint(&self) -> Result<(), DbError> {
        let conn = self.get_conn()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    // =========================================================================
    // Fusion+ Methods
    // =========================================================================

    /// Insert a new Fusion+ swap (from SrcEscrowCreated event)
    pub fn insert_fusion_plus_swap(&self, swap: &FusionPlusSwap) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "INSERT OR IGNORE INTO fusion_plus_swaps (
                order_hash, hashlock, secret,
                src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                src_escrow_address, src_maker, src_taker, src_token, src_amount,
                src_safety_deposit, src_timelocks, src_status,
                dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                dst_safety_deposit, dst_timelocks, dst_status,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31
            )",
            params![
                swap.order_hash.to_lowercase(),
                swap.hashlock.to_lowercase(),
                swap.secret,
                swap.src_chain_id,
                swap.src_tx_hash.to_lowercase(),
                swap.src_block_number,
                swap.src_block_timestamp,
                swap.src_log_index,
                swap.src_escrow_address.as_ref().map(|s| s.to_lowercase()),
                swap.src_maker.to_lowercase(),
                swap.src_taker.to_lowercase(),
                swap.src_token.to_lowercase(),
                swap.src_amount,
                swap.src_safety_deposit,
                swap.src_timelocks,
                swap.src_status,
                swap.dst_chain_id,
                swap.dst_tx_hash.as_ref().map(|s| s.to_lowercase()),
                swap.dst_block_number,
                swap.dst_block_timestamp,
                swap.dst_log_index,
                swap.dst_escrow_address.as_ref().map(|s| s.to_lowercase()),
                swap.dst_maker.to_lowercase(),
                swap.dst_taker.as_ref().map(|s| s.to_lowercase()),
                swap.dst_token.to_lowercase(),
                swap.dst_amount,
                swap.dst_safety_deposit,
                swap.dst_timelocks,
                swap.dst_status,
                now,
                now
            ],
        )?;

        Ok(result > 0)
    }

    /// Update swap with destination data (from DstEscrowCreated event)
    pub fn update_fusion_plus_dst(
        &self,
        order_hash: &str,
        dst_data: &DstEscrowCreatedData,
        chain_id: u32,
        tx_hash: &str,
        block_number: u64,
        block_timestamp: u64,
        log_index: u32,
        escrow_address: Option<&str>,
    ) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "UPDATE fusion_plus_swaps SET
                dst_tx_hash = ?1,
                dst_block_number = ?2,
                dst_block_timestamp = ?3,
                dst_log_index = ?4,
                dst_escrow_address = ?5,
                dst_taker = ?6,
                dst_timelocks = ?7,
                dst_status = 'created',
                updated_at = ?8
             WHERE order_hash = ?9 AND dst_chain_id = ?10",
            params![
                tx_hash.to_lowercase(),
                block_number,
                block_timestamp,
                log_index,
                escrow_address.map(|s| s.to_lowercase()),
                dst_data.dst_taker.to_lowercase(),
                dst_data.dst_timelocks,
                now,
                order_hash.to_lowercase(),
                chain_id
            ],
        )?;

        Ok(result > 0)
    }

    /// Update swap status on withdrawal
    pub fn update_fusion_plus_withdrawal(
        &self,
        order_hash: &str,
        chain_id: u32,
        is_src: bool,
        secret: &str,
    ) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sql = if is_src {
            "UPDATE fusion_plus_swaps SET
                src_status = 'withdrawn',
                secret = ?1,
                updated_at = ?2
             WHERE order_hash = ?3 AND src_chain_id = ?4"
        } else {
            "UPDATE fusion_plus_swaps SET
                dst_status = 'withdrawn',
                secret = ?1,
                updated_at = ?2
             WHERE order_hash = ?3 AND dst_chain_id = ?4"
        };

        let result = conn.execute(
            sql,
            params![
                secret.to_lowercase(),
                now,
                order_hash.to_lowercase(),
                chain_id
            ],
        )?;

        Ok(result > 0)
    }

    /// Update swap status on cancellation
    pub fn update_fusion_plus_cancelled(
        &self,
        order_hash: &str,
        chain_id: u32,
        is_src: bool,
    ) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sql = if is_src {
            "UPDATE fusion_plus_swaps SET
                src_status = 'cancelled',
                updated_at = ?1
             WHERE order_hash = ?2 AND src_chain_id = ?3"
        } else {
            "UPDATE fusion_plus_swaps SET
                dst_status = 'cancelled',
                updated_at = ?1
             WHERE order_hash = ?2 AND dst_chain_id = ?3"
        };

        let result = conn.execute(
            sql,
            params![now, order_hash.to_lowercase(), chain_id],
        )?;

        Ok(result > 0)
    }

    /// Update swap status on withdrawal by hashlock
    pub fn update_fusion_plus_withdrawal_by_hashlock(
        &self,
        hashlock: &str,
        chain_id: u32,
        is_src: bool,
        secret: &str,
        tx_hash: &str,
        block_number: u64,
        block_timestamp: u64,
        log_index: u32,
    ) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = if is_src {
            conn.execute(
                "UPDATE fusion_plus_swaps SET
                    src_status = 'withdrawn',
                    secret = ?1,
                    updated_at = ?2
                 WHERE hashlock = ?3 AND src_chain_id = ?4",
                params![
                    secret.to_lowercase(),
                    now,
                    hashlock.to_lowercase(),
                    chain_id
                ],
            )?
        } else {
            conn.execute(
                "UPDATE fusion_plus_swaps SET
                    dst_status = 'withdrawn',
                    dst_tx_hash = ?5,
                    dst_block_number = ?6,
                    dst_block_timestamp = ?7,
                    dst_log_index = ?8,
                    secret = ?1,
                    updated_at = ?2
                 WHERE hashlock = ?3 AND dst_chain_id = ?4",
                params![
                    secret.to_lowercase(),
                    now,
                    hashlock.to_lowercase(),
                    chain_id,
                    tx_hash.to_lowercase(),
                    block_number,
                    block_timestamp,
                    log_index
                ],
            )?
        };

        Ok(result > 0)
    }

    /// Get Fusion+ swap by order_hash
    pub fn get_fusion_plus_swap(&self, order_hash: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let conn = self.get_conn()?;

        let result = conn.query_row(
            "SELECT order_hash, hashlock, secret,
                    src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                    src_escrow_address, src_maker, src_taker, src_token, src_amount,
                    src_safety_deposit, src_timelocks, src_status,
                    dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                    dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                    dst_safety_deposit, dst_timelocks, dst_status
             FROM fusion_plus_swaps WHERE order_hash = ?1",
            params![order_hash.to_lowercase()],
            |row| {
                Ok(FusionPlusSwap {
                    order_hash: row.get(0)?,
                    hashlock: row.get(1)?,
                    secret: row.get(2)?,
                    src_chain_id: row.get(3)?,
                    src_tx_hash: row.get(4)?,
                    src_block_number: row.get(5)?,
                    src_block_timestamp: row.get(6)?,
                    src_log_index: row.get(7)?,
                    src_escrow_address: row.get(8)?,
                    src_maker: row.get(9)?,
                    src_taker: row.get(10)?,
                    src_token: row.get(11)?,
                    src_amount: row.get(12)?,
                    src_safety_deposit: row.get(13)?,
                    src_timelocks: row.get(14)?,
                    src_status: row.get(15)?,
                    dst_chain_id: row.get(16)?,
                    dst_tx_hash: row.get(17)?,
                    dst_block_number: row.get(18)?,
                    dst_block_timestamp: row.get(19)?,
                    dst_log_index: row.get(20)?,
                    dst_escrow_address: row.get(21)?,
                    dst_maker: row.get(22)?,
                    dst_taker: row.get(23)?,
                    dst_token: row.get(24)?,
                    dst_amount: row.get(25)?,
                    dst_safety_deposit: row.get(26)?,
                    dst_timelocks: row.get(27)?,
                    dst_status: row.get(28)?,
                })
            },
        );

        match result {
            Ok(swap) => Ok(Some(swap)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get Fusion+ swap by hashlock
    pub fn get_fusion_plus_swap_by_hashlock(&self, hashlock: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let conn = self.get_conn()?;

        let result = conn.query_row(
            "SELECT order_hash, hashlock, secret,
                    src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                    src_escrow_address, src_maker, src_taker, src_token, src_amount,
                    src_safety_deposit, src_timelocks, src_status,
                    dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                    dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                    dst_safety_deposit, dst_timelocks, dst_status
             FROM fusion_plus_swaps WHERE hashlock = ?1",
            params![hashlock.to_lowercase()],
            |row| {
                Ok(FusionPlusSwap {
                    order_hash: row.get(0)?,
                    hashlock: row.get(1)?,
                    secret: row.get(2)?,
                    src_chain_id: row.get(3)?,
                    src_tx_hash: row.get(4)?,
                    src_block_number: row.get(5)?,
                    src_block_timestamp: row.get(6)?,
                    src_log_index: row.get(7)?,
                    src_escrow_address: row.get(8)?,
                    src_maker: row.get(9)?,
                    src_taker: row.get(10)?,
                    src_token: row.get(11)?,
                    src_amount: row.get(12)?,
                    src_safety_deposit: row.get(13)?,
                    src_timelocks: row.get(14)?,
                    src_status: row.get(15)?,
                    dst_chain_id: row.get(16)?,
                    dst_tx_hash: row.get(17)?,
                    dst_block_number: row.get(18)?,
                    dst_block_timestamp: row.get(19)?,
                    dst_log_index: row.get(20)?,
                    dst_escrow_address: row.get(21)?,
                    dst_maker: row.get(22)?,
                    dst_taker: row.get(23)?,
                    dst_token: row.get(24)?,
                    dst_amount: row.get(25)?,
                    dst_safety_deposit: row.get(26)?,
                    dst_timelocks: row.get(27)?,
                    dst_status: row.get(28)?,
                })
            },
        );

        match result {
            Ok(swap) => Ok(Some(swap)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get total count of Fusion+ swaps
    pub fn get_fusion_plus_count(&self) -> Result<u64, DbError> {
        let conn = self.get_conn()?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM fusion_plus_swaps",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Fusion+ swaps based on TTL
    pub fn cleanup_old_fusion_plus(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.get_conn()?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - ttl_secs;

        let deleted = conn.execute(
            "DELETE FROM fusion_plus_swaps WHERE created_at < ?1",
            params![cutoff],
        )?;

        Ok(deleted)
    }

    // =========================================================================
    // Fusion (Single-Chain) Methods
    // =========================================================================

    /// Insert a new Fusion swap
    pub fn insert_fusion_swap(&self, swap: &FusionSwap) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "INSERT OR IGNORE INTO fusion_swaps (
                order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                remaining, is_partial_fill, status, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                swap.order_hash.to_lowercase(),
                swap.chain_id,
                swap.tx_hash.to_lowercase(),
                swap.block_number,
                swap.block_timestamp,
                swap.log_index,
                swap.maker.to_lowercase(),
                swap.taker.as_ref().map(|s| s.to_lowercase()),
                swap.maker_token.as_ref().map(|s| s.to_lowercase()),
                swap.taker_token.as_ref().map(|s| s.to_lowercase()),
                swap.maker_amount,
                swap.taker_amount,
                swap.remaining,
                swap.is_partial_fill as i32,
                swap.status,
                now
            ],
        )?;

        Ok(result > 0)
    }

    /// Get Fusion swap by order_hash
    pub fn get_fusion_swap_by_order_hash(&self, order_hash: &str) -> Result<Option<FusionSwap>, DbError> {
        let conn = self.get_conn()?;

        let result = conn.query_row(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE order_hash = ?1
             ORDER BY block_timestamp DESC LIMIT 1",
            params![order_hash.to_lowercase()],
            |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            },
        );

        match result {
            Ok(swap) => Ok(Some(swap)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get Fusion swaps by maker address
    pub fn get_fusion_swaps_by_maker(&self, maker: &str, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE maker = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let swaps = stmt
            .query_map(params![maker.to_lowercase(), limit], |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(swaps)
    }

    /// Get Fusion swaps by taker address
    pub fn get_fusion_swaps_by_taker(&self, taker: &str, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE taker = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let swaps = stmt
            .query_map(params![taker.to_lowercase(), limit], |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(swaps)
    }

    /// Get Fusion swaps by chain
    pub fn get_fusion_swaps_by_chain(&self, chain_id: u32, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE chain_id = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let swaps = stmt
            .query_map(params![chain_id, limit], |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(swaps)
    }

    /// Get Fusion swaps by status
    pub fn get_fusion_swaps_by_status(&self, status: &str, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE status = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let swaps = stmt
            .query_map(params![status, limit], |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(swaps)
    }

    /// Get recent Fusion swaps
    pub fn get_recent_fusion_swaps(&self, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps
             ORDER BY block_timestamp DESC LIMIT ?1"
        )?;

        let swaps = stmt
            .query_map(params![limit], |row| {
                Ok(FusionSwap {
                    order_hash: row.get(0)?,
                    chain_id: row.get(1)?,
                    tx_hash: row.get(2)?,
                    block_number: row.get(3)?,
                    block_timestamp: row.get(4)?,
                    log_index: row.get(5)?,
                    maker: row.get(6)?,
                    taker: row.get(7)?,
                    maker_token: row.get(8)?,
                    taker_token: row.get(9)?,
                    maker_amount: row.get(10)?,
                    taker_amount: row.get(11)?,
                    remaining: row.get(12)?,
                    is_partial_fill: row.get::<_, i32>(13)? != 0,
                    status: row.get(14)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(swaps)
    }

    /// Get total count of Fusion swaps
    pub fn get_fusion_swap_count(&self) -> Result<u64, DbError> {
        let conn = self.get_conn()?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM fusion_swaps",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Fusion swaps based on TTL
    pub fn cleanup_old_fusion_swaps(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.get_conn()?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - ttl_secs;

        let deleted = conn.execute(
            "DELETE FROM fusion_swaps WHERE created_at < ?1",
            params![cutoff],
        )?;

        Ok(deleted)
    }

    // =========================================================================
    // Crypto2Fiat Methods
    // =========================================================================

    /// Insert a new Crypto2Fiat event
    pub fn insert_crypto2fiat_event(&self, event: &Crypto2FiatEvent) -> Result<bool, DbError> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = conn.execute(
            "INSERT OR IGNORE INTO crypto2fiat_events (
                order_id, token, amount, recipient, metadata,
                chain_id, tx_hash, block_number, block_timestamp, log_index, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                event.order_id.to_lowercase(),
                event.token.to_lowercase(),
                event.amount,
                event.recipient.to_lowercase(),
                event.metadata,
                event.chain_id,
                event.tx_hash.to_lowercase(),
                event.block_number,
                event.block_timestamp,
                event.log_index,
                now
            ],
        )?;

        Ok(result > 0)
    }

    /// Get Crypto2Fiat event by order_id
    pub fn get_crypto2fiat_by_order_id(&self, order_id: &str) -> Result<Option<Crypto2FiatEvent>, DbError> {
        let conn = self.get_conn()?;

        let result = conn.query_row(
            "SELECT order_id, token, amount, recipient, metadata,
                    chain_id, tx_hash, block_number, block_timestamp, log_index
             FROM crypto2fiat_events WHERE order_id = ?1",
            params![order_id.to_lowercase()],
            |row| {
                Ok(Crypto2FiatEvent {
                    order_id: row.get(0)?,
                    token: row.get(1)?,
                    amount: row.get(2)?,
                    recipient: row.get(3)?,
                    metadata: row.get(4)?,
                    chain_id: row.get(5)?,
                    tx_hash: row.get(6)?,
                    block_number: row.get(7)?,
                    block_timestamp: row.get(8)?,
                    log_index: row.get(9)?,
                })
            },
        );

        match result {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get Crypto2Fiat events by recipient address
    pub fn get_crypto2fiat_by_recipient(&self, recipient: &str, limit: u32) -> Result<Vec<Crypto2FiatEvent>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_id, token, amount, recipient, metadata,
                    chain_id, tx_hash, block_number, block_timestamp, log_index
             FROM crypto2fiat_events WHERE recipient = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let events = stmt
            .query_map(params![recipient.to_lowercase(), limit], |row| {
                Ok(Crypto2FiatEvent {
                    order_id: row.get(0)?,
                    token: row.get(1)?,
                    amount: row.get(2)?,
                    recipient: row.get(3)?,
                    metadata: row.get(4)?,
                    chain_id: row.get(5)?,
                    tx_hash: row.get(6)?,
                    block_number: row.get(7)?,
                    block_timestamp: row.get(8)?,
                    log_index: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(events)
    }

    /// Get Crypto2Fiat events by chain
    pub fn get_crypto2fiat_by_chain(&self, chain_id: u32, limit: u32) -> Result<Vec<Crypto2FiatEvent>, DbError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT order_id, token, amount, recipient, metadata,
                    chain_id, tx_hash, block_number, block_timestamp, log_index
             FROM crypto2fiat_events WHERE chain_id = ?1
             ORDER BY block_timestamp DESC LIMIT ?2"
        )?;

        let events = stmt
            .query_map(params![chain_id, limit], |row| {
                Ok(Crypto2FiatEvent {
                    order_id: row.get(0)?,
                    token: row.get(1)?,
                    amount: row.get(2)?,
                    recipient: row.get(3)?,
                    metadata: row.get(4)?,
                    chain_id: row.get(5)?,
                    tx_hash: row.get(6)?,
                    block_number: row.get(7)?,
                    block_timestamp: row.get(8)?,
                    log_index: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(events)
    }

    /// Get total count of Crypto2Fiat events
    pub fn get_crypto2fiat_count(&self) -> Result<u64, DbError> {
        let conn = self.get_conn()?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM crypto2fiat_events",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Crypto2Fiat events based on TTL
    pub fn cleanup_old_crypto2fiat(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.get_conn()?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - ttl_secs;

        let deleted = conn.execute(
            "DELETE FROM crypto2fiat_events WHERE created_at < ?1",
            params![cutoff],
        )?;

        Ok(deleted)
    }
}

// =============================================================================
// DatabaseManager - Manages all chain databases and shared database
// =============================================================================

/// Manages all chain databases and the shared database
pub struct DatabaseManager {
    chains: HashMap<u32, Arc<ChainDatabase>>,
    shared: Arc<SharedDatabase>,
}

impl DatabaseManager {
    /// Open all chain databases and the shared database
    pub fn open(data_dir: &str, chain_ids: &[u32]) -> Result<Self, DbError> {
        let mut chains = HashMap::new();

        for &chain_id in chain_ids {
            let db = ChainDatabase::open(data_dir, chain_id)?;
            chains.insert(chain_id, Arc::new(db));
        }

        let shared = SharedDatabase::open(data_dir)?;

        Ok(Self {
            chains,
            shared: Arc::new(shared),
        })
    }

    /// Get the database for a specific chain
    pub fn chain(&self, chain_id: u32) -> Option<Arc<ChainDatabase>> {
        self.chains.get(&chain_id).cloned()
    }

    /// Get the shared database
    pub fn shared(&self) -> Arc<SharedDatabase> {
        Arc::clone(&self.shared)
    }

    /// Get all chain IDs
    pub fn chain_ids(&self) -> Vec<u32> {
        self.chains.keys().copied().collect()
    }

    /// Force WAL checkpoint on all databases
    pub fn checkpoint_all(&self) -> Result<(), DbError> {
        for db in self.chains.values() {
            db.checkpoint()?;
        }
        self.shared.checkpoint()?;
        Ok(())
    }

    /// Clean up old data from all databases
    pub fn cleanup_all(&self, ttl_secs: u64) -> Result<CleanupStats, DbError> {
        let mut stats = CleanupStats::default();

        for db in self.chains.values() {
            stats.transfers_deleted += db.cleanup_old(ttl_secs)?;
        }

        stats.fusion_plus_deleted += self.shared.cleanup_old_fusion_plus(ttl_secs)?;
        stats.fusion_deleted += self.shared.cleanup_old_fusion_swaps(ttl_secs)?;
        stats.crypto2fiat_deleted += self.shared.cleanup_old_crypto2fiat(ttl_secs)?;

        Ok(stats)
    }

    /// Get total transfer count across all chains
    pub fn get_total_transfer_count(&self) -> Result<u64, DbError> {
        let mut total = 0;
        for db in self.chains.values() {
            total += db.get_transfer_count()?;
        }
        Ok(total)
    }
}

#[derive(Default, Debug)]
pub struct CleanupStats {
    pub transfers_deleted: usize,
    pub fusion_plus_deleted: usize,
    pub fusion_deleted: usize,
    pub crypto2fiat_deleted: usize,
}
