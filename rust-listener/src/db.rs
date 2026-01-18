use crate::types::{Crypto2FiatEvent, DstEscrowCreatedData, FusionPlusSwap, FusionSwap, Transfer};
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

        // Add swap_type column if it doesn't exist (for existing databases)
        // SQLite doesn't have ADD COLUMN IF NOT EXISTS, so we check first
        let has_swap_type: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('transfers') WHERE name = 'swap_type'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_swap_type {
            conn.execute("ALTER TABLE transfers ADD COLUMN swap_type TEXT", [])?;
        }

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
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_swap_type ON transfers(chain_id, swap_type, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tx_hash ON transfers(chain_id, tx_hash)",
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

        // Create fusion_plus_swaps table for 1inch Fusion+ cross-chain swaps
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fusion_plus_swaps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,

                -- Correlation keys (SAME on both chains)
                order_hash TEXT NOT NULL UNIQUE,
                hashlock TEXT NOT NULL,
                secret TEXT,

                -- Source chain data (from SrcEscrowCreated event)
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

                -- Destination chain data (NULLABLE until DstEscrowCreated)
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

                -- Metadata
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create indexes for fusion_plus_swaps
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_hashlock ON fusion_plus_swaps(hashlock)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_src_chain ON fusion_plus_swaps(src_chain_id, src_block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_chain ON fusion_plus_swaps(dst_chain_id, dst_block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_src_maker ON fusion_plus_swaps(src_maker)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_maker ON fusion_plus_swaps(dst_maker)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_src_taker ON fusion_plus_swaps(src_taker)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_status ON fusion_plus_swaps(src_status, dst_status)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fp_created ON fusion_plus_swaps(created_at)",
            [],
        )?;

        // Create fusion_swaps table for 1inch Fusion single-chain swaps
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

        // Add taker column if it doesn't exist (for existing databases)
        let _ = conn.execute("ALTER TABLE fusion_swaps ADD COLUMN taker TEXT", []);

        // Create indexes for fusion_swaps
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_order_hash ON fusion_swaps(order_hash)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_chain ON fusion_swaps(chain_id, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_maker ON fusion_swaps(maker)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_taker ON fusion_swaps(taker)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_status ON fusion_swaps(status)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_created ON fusion_swaps(created_at)",
            [],
        )?;

        // Create crypto2fiat_events table for KentuckyDelegate crypto-to-fiat offramps
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

        // Create indexes for crypto2fiat_events
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_c2f_order_id ON crypto2fiat_events(order_id)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_c2f_token ON crypto2fiat_events(token)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_c2f_recipient ON crypto2fiat_events(recipient)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_c2f_chain ON crypto2fiat_events(chain_id, block_timestamp DESC)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_c2f_created ON crypto2fiat_events(created_at)",
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

    /// Get transfers by transaction hash (for enriching Fusion swap data)
    pub fn get_transfers_by_tx_hash(
        &self,
        chain_id: u32,
        tx_hash: &str,
    ) -> Result<Vec<Transfer>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp
             FROM transfers
             WHERE chain_id = ?1 AND tx_hash = ?2
             ORDER BY log_index ASC"
        )?;

        let transfers = stmt
            .query_map(params![chain_id, tx_hash.to_lowercase()], |row| {
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

    // =========================================================================
    // Fusion+ Methods
    // =========================================================================

    /// Insert a new Fusion+ swap (from SrcEscrowCreated event)
    pub fn insert_fusion_plus_swap(&self, swap: &FusionPlusSwap) -> Result<bool, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Update swap status on withdrawal (updates secret and status)
    pub fn update_fusion_plus_withdrawal(
        &self,
        order_hash: &str,
        chain_id: u32,
        is_src: bool,
        secret: &str,
    ) -> Result<bool, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Update swap status on withdrawal by hashlock (for when we don't know order_hash)
    /// This is used when processing EscrowWithdrawal events
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sql = if is_src {
            "UPDATE fusion_plus_swaps SET
                src_status = 'withdrawn',
                secret = ?1,
                updated_at = ?2
             WHERE hashlock = ?3 AND src_chain_id = ?4"
        } else {
            "UPDATE fusion_plus_swaps SET
                dst_status = 'withdrawn',
                dst_tx_hash = ?5,
                dst_block_number = ?6,
                dst_block_timestamp = ?7,
                dst_log_index = ?8,
                secret = ?1,
                updated_at = ?2
             WHERE hashlock = ?3 AND dst_chain_id = ?4"
        };

        let result = if is_src {
            conn.execute(
                sql,
                params![
                    secret.to_lowercase(),
                    now,
                    hashlock.to_lowercase(),
                    chain_id
                ],
            )?
        } else {
            conn.execute(
                sql,
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

    /// Label transfers in a transaction as fusion_plus
    pub fn label_transfers_as_fusion(
        &self,
        chain_id: u32,
        tx_hash: &str,
        swap_type: &str,
    ) -> Result<usize, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;

        let result = conn.execute(
            "UPDATE transfers SET swap_type = ?1 WHERE chain_id = ?2 AND tx_hash = ?3",
            params![swap_type, chain_id, tx_hash.to_lowercase()],
        )?;

        Ok(result)
    }

    /// Get Fusion+ swap by order_hash
    pub fn get_fusion_plus_swap(&self, order_hash: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;

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

    /// Get Fusion+ swap by hashlock (for matching withdrawal events)
    pub fn get_fusion_plus_swap_by_hashlock(&self, hashlock: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;

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

    /// Get total count of Fusion+ swaps (for monitoring)
    pub fn get_fusion_plus_count(&self) -> Result<u64, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM fusion_plus_swaps",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Fusion+ swaps based on TTL
    pub fn cleanup_old_fusion_plus(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Insert a new Fusion swap (from OrderFilled or OrderCancelled event)
    pub fn insert_fusion_swap(&self, swap: &FusionSwap) -> Result<bool, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;

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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Get Fusion swaps by chain
    pub fn get_fusion_swaps_by_chain(&self, chain_id: u32, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Get Fusion swaps by taker address
    pub fn get_fusion_swaps_by_taker(&self, taker: &str, limit: u32) -> Result<Vec<FusionSwap>, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Get total count of Fusion swaps (for monitoring)
    pub fn get_fusion_swap_count(&self) -> Result<u64, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM fusion_swaps",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Fusion swaps based on TTL
    pub fn cleanup_old_fusion_swaps(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;

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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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

    /// Label transfers in a transaction as crypto_to_fiat
    pub fn label_transfers_as_crypto2fiat(&self, chain_id: u32, tx_hash: &str) -> Result<usize, DbError> {
        self.label_transfers_as_fusion(chain_id, tx_hash, "crypto_to_fiat")
    }

    /// Get total count of Crypto2Fiat events (for monitoring)
    pub fn get_crypto2fiat_count(&self) -> Result<u64, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM crypto2fiat_events",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Clean up old Crypto2Fiat events based on TTL
    pub fn cleanup_old_crypto2fiat(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
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
