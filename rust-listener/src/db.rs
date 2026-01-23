use crate::types::{Crypto2FiatEvent, DstEscrowCreatedData, FusionPlusSwap, FusionSwap, Transfer};
use deadpool_postgres::{Config, Pool, Runtime, PoolError};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio_postgres::{NoTls, Row};

#[derive(Error, Debug)]
pub enum DbError {
    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    #[error("Pool error: {0}")]
    Pool(#[from] PoolError),
    #[error("Configuration error: {0}")]
    Config(String),
}

/// PostgreSQL Database with connection pool
/// All chains share a single database with chain_id column
pub struct Database {
    pool: Pool,
}

impl Database {
    /// Create a new database connection pool from DATABASE_URL
    pub async fn new(database_url: &str) -> Result<Self, DbError> {
        // Parse the DATABASE_URL
        let config = database_url
            .parse::<tokio_postgres::Config>()
            .map_err(|e| DbError::Config(e.to_string()))?;

        // Build deadpool config
        let mut cfg = Config::new();
        cfg.host = config.get_hosts().first().map(|h| match h {
            tokio_postgres::config::Host::Tcp(s) => s.clone(),
            tokio_postgres::config::Host::Unix(p) => p.to_string_lossy().to_string(),
        });
        cfg.port = config.get_ports().first().copied();
        cfg.user = config.get_user().map(|s| s.to_string());
        cfg.password = config.get_password().map(|s| String::from_utf8_lossy(s).to_string());
        cfg.dbname = config.get_dbname().map(|s| s.to_string());

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| DbError::Config(e.to_string()))?;

        let db = Self { pool };

        // Auto-create schema on startup
        db.create_schema().await?;

        Ok(db)
    }

    /// Create all tables and indexes if they don't exist
    async fn create_schema(&self) -> Result<(), DbError> {
        let client = self.pool.get().await?;

        // Transfers table (chain-specific data with chain_id column)
        client.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                id BIGSERIAL PRIMARY KEY,
                chain_id INTEGER NOT NULL,
                tx_hash VARCHAR(66) NOT NULL,
                log_index INTEGER NOT NULL,
                token VARCHAR(42) NOT NULL,
                from_addr VARCHAR(42) NOT NULL,
                to_addr VARCHAR(42) NOT NULL,
                value VARCHAR(78) NOT NULL,
                block_number BIGINT NOT NULL,
                block_timestamp BIGINT NOT NULL,
                swap_type VARCHAR(20),
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            &[],
        ).await?;

        // Checkpoints table (one row per chain)
        client.execute(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                chain_id INTEGER PRIMARY KEY,
                block_number BIGINT NOT NULL,
                updated_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
            )",
            &[],
        ).await?;

        // Fusion+ swaps table
        client.execute(
            "CREATE TABLE IF NOT EXISTS fusion_plus_swaps (
                id BIGSERIAL PRIMARY KEY,
                order_hash VARCHAR(66) NOT NULL UNIQUE,
                hashlock VARCHAR(66) NOT NULL,
                secret VARCHAR(66),
                src_chain_id INTEGER NOT NULL,
                src_tx_hash VARCHAR(66) NOT NULL,
                src_block_number BIGINT NOT NULL,
                src_block_timestamp BIGINT NOT NULL,
                src_log_index INTEGER NOT NULL,
                src_escrow_address VARCHAR(42),
                src_maker VARCHAR(42) NOT NULL,
                src_taker VARCHAR(42) NOT NULL,
                src_token VARCHAR(42) NOT NULL,
                src_amount VARCHAR(78) NOT NULL,
                src_safety_deposit VARCHAR(78) NOT NULL,
                src_timelocks VARCHAR(130) NOT NULL,
                src_status VARCHAR(20) NOT NULL DEFAULT 'created',
                dst_chain_id INTEGER NOT NULL,
                dst_tx_hash VARCHAR(66),
                dst_block_number BIGINT,
                dst_block_timestamp BIGINT,
                dst_log_index INTEGER,
                dst_escrow_address VARCHAR(42),
                dst_maker VARCHAR(42) NOT NULL,
                dst_taker VARCHAR(42),
                dst_token VARCHAR(42) NOT NULL,
                dst_amount VARCHAR(78) NOT NULL,
                dst_safety_deposit VARCHAR(78) NOT NULL,
                dst_timelocks VARCHAR(130),
                dst_status VARCHAR(20) NOT NULL DEFAULT 'pending',
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                updated_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
            )",
            &[],
        ).await?;

        // Fusion swaps table (single-chain)
        client.execute(
            "CREATE TABLE IF NOT EXISTS fusion_swaps (
                id BIGSERIAL PRIMARY KEY,
                order_hash VARCHAR(66) NOT NULL,
                chain_id INTEGER NOT NULL,
                tx_hash VARCHAR(66) NOT NULL,
                block_number BIGINT NOT NULL,
                block_timestamp BIGINT NOT NULL,
                log_index INTEGER NOT NULL,
                maker VARCHAR(42) NOT NULL,
                taker VARCHAR(42),
                maker_token VARCHAR(42),
                taker_token VARCHAR(42),
                maker_amount VARCHAR(78),
                taker_amount VARCHAR(78),
                remaining VARCHAR(78) NOT NULL,
                is_partial_fill BOOLEAN NOT NULL DEFAULT FALSE,
                status VARCHAR(20) NOT NULL DEFAULT 'filled',
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            &[],
        ).await?;

        // Crypto2Fiat events table
        client.execute(
            "CREATE TABLE IF NOT EXISTS crypto2fiat_events (
                id BIGSERIAL PRIMARY KEY,
                order_id VARCHAR(66) NOT NULL,
                token VARCHAR(42) NOT NULL,
                amount VARCHAR(78) NOT NULL,
                recipient VARCHAR(42) NOT NULL,
                metadata TEXT,
                chain_id INTEGER NOT NULL,
                tx_hash VARCHAR(66) NOT NULL,
                block_number BIGINT NOT NULL,
                block_timestamp BIGINT NOT NULL,
                log_index INTEGER NOT NULL,
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                UNIQUE(chain_id, tx_hash, log_index)
            )",
            &[],
        ).await?;

        // Create indexes for transfers
        let transfer_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_transfers_from ON transfers(chain_id, from_addr, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_to ON transfers(chain_id, to_addr, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_tx_hash ON transfers(chain_id, tx_hash)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_created ON transfers(created_at)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_swap_type ON transfers(chain_id, swap_type, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_from_id ON transfers(chain_id, from_addr, id)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_to_id ON transfers(chain_id, to_addr, id)",
        ];

        for sql in transfer_indexes {
            client.execute(sql, &[]).await?;
        }

        // Create indexes for fusion_plus_swaps
        let fp_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_fp_hashlock ON fusion_plus_swaps(hashlock)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_chain ON fusion_plus_swaps(src_chain_id, src_block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_chain ON fusion_plus_swaps(dst_chain_id, dst_block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_maker ON fusion_plus_swaps(src_maker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_maker ON fusion_plus_swaps(dst_maker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_taker ON fusion_plus_swaps(src_taker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_status ON fusion_plus_swaps(src_status, dst_status)",
            "CREATE INDEX IF NOT EXISTS idx_fp_created ON fusion_plus_swaps(created_at)",
        ];

        for sql in fp_indexes {
            client.execute(sql, &[]).await?;
        }

        // Create indexes for fusion_swaps
        let fs_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_fs_order_hash ON fusion_swaps(order_hash)",
            "CREATE INDEX IF NOT EXISTS idx_fs_chain ON fusion_swaps(chain_id, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fs_maker ON fusion_swaps(maker)",
            "CREATE INDEX IF NOT EXISTS idx_fs_taker ON fusion_swaps(taker)",
            "CREATE INDEX IF NOT EXISTS idx_fs_status ON fusion_swaps(status)",
            "CREATE INDEX IF NOT EXISTS idx_fs_created ON fusion_swaps(created_at)",
        ];

        for sql in fs_indexes {
            client.execute(sql, &[]).await?;
        }

        // Create indexes for crypto2fiat_events
        let c2f_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_c2f_order_id ON crypto2fiat_events(order_id)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_token ON crypto2fiat_events(token)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_recipient ON crypto2fiat_events(recipient)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_chain ON crypto2fiat_events(chain_id, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_created ON crypto2fiat_events(created_at)",
        ];

        for sql in c2f_indexes {
            client.execute(sql, &[]).await?;
        }

        tracing::info!("PostgreSQL schema initialized");
        Ok(())
    }

    // =========================================================================
    // Transfer Methods
    // =========================================================================

    /// Insert a transfer, ignoring duplicates
    pub async fn insert_transfer(&self, chain_id: u32, transfer: &Transfer) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = client.execute(
            "INSERT INTO transfers
             (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
            &[
                &(chain_id as i32),
                &transfer.tx_hash.to_lowercase(),
                &(transfer.log_index as i32),
                &transfer.token.to_lowercase(),
                &transfer.from_addr.to_lowercase(),
                &transfer.to_addr.to_lowercase(),
                &transfer.value,
                &(transfer.block_number as i64),
                &(transfer.block_timestamp as i64),
                &transfer.swap_type,
                &now,
            ],
        ).await?;

        Ok(result > 0)
    }

    /// Insert multiple transfers in a batch
    pub async fn insert_transfers_batch(&self, chain_id: u32, transfers: &[Transfer]) -> Result<usize, DbError> {
        if transfers.is_empty() {
            return Ok(0);
        }

        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let stmt = client.prepare(
            "INSERT INTO transfers
             (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING"
        ).await?;

        let mut inserted = 0;
        for transfer in transfers {
            let result = client.execute(
                &stmt,
                &[
                    &(chain_id as i32),
                    &transfer.tx_hash.to_lowercase(),
                    &(transfer.log_index as i32),
                    &transfer.token.to_lowercase(),
                    &transfer.from_addr.to_lowercase(),
                    &transfer.to_addr.to_lowercase(),
                    &transfer.value,
                    &(transfer.block_number as i64),
                    &(transfer.block_timestamp as i64),
                    &transfer.swap_type,
                    &now,
                ],
            ).await?;
            if result > 0 {
                inserted += 1;
            }
        }

        Ok(inserted)
    }

    /// Get checkpoint block number for a chain
    pub async fn get_checkpoint(&self, chain_id: u32) -> Result<Option<u64>, DbError> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT block_number FROM checkpoints WHERE chain_id = $1",
            &[&(chain_id as i32)],
        ).await?;

        Ok(row.map(|r| r.get::<_, i64>(0) as u64))
    }

    /// Set checkpoint block number for a chain
    pub async fn set_checkpoint(&self, chain_id: u32, block_number: u64) -> Result<(), DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        client.execute(
            "INSERT INTO checkpoints (chain_id, block_number, updated_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (chain_id) DO UPDATE SET
             block_number = EXCLUDED.block_number,
             updated_at = EXCLUDED.updated_at",
            &[&(chain_id as i32), &(block_number as i64), &now],
        ).await?;

        Ok(())
    }

    /// Clean up old transfers based on TTL
    pub async fn cleanup_old_transfers(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - ttl_secs as i64;

        let deleted = client.execute(
            "DELETE FROM transfers WHERE created_at < $1",
            &[&cutoff],
        ).await?;

        Ok(deleted as usize)
    }

    /// Get total count of transfers for a chain
    pub async fn get_transfer_count(&self, chain_id: u32) -> Result<u64, DbError> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "SELECT COUNT(*) FROM transfers WHERE chain_id = $1",
            &[&(chain_id as i32)],
        ).await?;

        Ok(row.get::<_, i64>(0) as u64)
    }

    /// Get total transfer count across all chains
    pub async fn get_total_transfer_count(&self) -> Result<u64, DbError> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "SELECT COUNT(*) FROM transfers",
            &[],
        ).await?;

        Ok(row.get::<_, i64>(0) as u64)
    }

    /// Label transfers in a transaction with swap_type
    pub async fn label_transfers_as_fusion(&self, chain_id: u32, tx_hash: &str, swap_type: &str) -> Result<usize, DbError> {
        let client = self.pool.get().await?;

        let result = client.execute(
            "UPDATE transfers SET swap_type = $1 WHERE chain_id = $2 AND tx_hash = $3",
            &[&swap_type, &(chain_id as i32), &tx_hash.to_lowercase()],
        ).await?;

        Ok(result as usize)
    }

    /// Get first and last transfers for a transaction (by log_index)
    /// Returns (first_transfer, last_transfer) for populating swap maker/taker info
    pub async fn get_first_last_transfers(&self, chain_id: u32, tx_hash: &str) -> Result<Option<(Transfer, Transfer)>, DbError> {
        let client = self.pool.get().await?;
        let tx_hash_lower = tx_hash.to_lowercase();

        // Get first transfer (lowest log_index)
        let first_row = client.query_opt(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type
             FROM transfers
             WHERE chain_id = $1 AND tx_hash = $2
             ORDER BY log_index ASC
             LIMIT 1",
            &[&(chain_id as i32), &tx_hash_lower],
        ).await?;

        // Get last transfer (highest log_index)
        let last_row = client.query_opt(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type
             FROM transfers
             WHERE chain_id = $1 AND tx_hash = $2
             ORDER BY log_index DESC
             LIMIT 1",
            &[&(chain_id as i32), &tx_hash_lower],
        ).await?;

        match (first_row, last_row) {
            (Some(first), Some(last)) => {
                let first_transfer = Transfer {
                    chain_id,
                    tx_hash: first.get(0),
                    log_index: first.get::<_, i32>(1) as u32,
                    token: first.get(2),
                    from_addr: first.get(3),
                    to_addr: first.get(4),
                    value: first.get(5),
                    block_number: first.get::<_, i64>(6) as u64,
                    block_timestamp: first.get::<_, i64>(7) as u64,
                    swap_type: first.get(8),
                };
                let last_transfer = Transfer {
                    chain_id,
                    tx_hash: last.get(0),
                    log_index: last.get::<_, i32>(1) as u32,
                    token: last.get(2),
                    from_addr: last.get(3),
                    to_addr: last.get(4),
                    value: last.get(5),
                    block_number: last.get::<_, i64>(6) as u64,
                    block_timestamp: last.get::<_, i64>(7) as u64,
                    swap_type: last.get(8),
                };
                Ok(Some((first_transfer, last_transfer)))
            }
            _ => Ok(None),
        }
    }

    // =========================================================================
    // Fusion+ Methods
    // =========================================================================

    /// Insert a new Fusion+ swap
    pub async fn insert_fusion_plus_swap(&self, swap: &FusionPlusSwap) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = client.execute(
            "INSERT INTO fusion_plus_swaps (
                order_hash, hashlock, secret,
                src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                src_escrow_address, src_maker, src_taker, src_token, src_amount,
                src_safety_deposit, src_timelocks, src_status,
                dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                dst_safety_deposit, dst_timelocks, dst_status,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31
            )
            ON CONFLICT (order_hash) DO NOTHING",
            &[
                &swap.order_hash.to_lowercase(),
                &swap.hashlock.to_lowercase(),
                &swap.secret,
                &(swap.src_chain_id as i32),
                &swap.src_tx_hash.to_lowercase(),
                &(swap.src_block_number as i64),
                &(swap.src_block_timestamp as i64),
                &(swap.src_log_index as i32),
                &swap.src_escrow_address.as_ref().map(|s| s.to_lowercase()),
                &swap.src_maker.to_lowercase(),
                &swap.src_taker.to_lowercase(),
                &swap.src_token.to_lowercase(),
                &swap.src_amount,
                &swap.src_safety_deposit,
                &swap.src_timelocks,
                &swap.src_status,
                &(swap.dst_chain_id as i32),
                &swap.dst_tx_hash.as_ref().map(|s| s.to_lowercase()),
                &swap.dst_block_number.map(|n| n as i64),
                &swap.dst_block_timestamp.map(|n| n as i64),
                &swap.dst_log_index.map(|n| n as i32),
                &swap.dst_escrow_address.as_ref().map(|s| s.to_lowercase()),
                &swap.dst_maker.to_lowercase(),
                &swap.dst_taker.as_ref().map(|s| s.to_lowercase()),
                &swap.dst_token.to_lowercase(),
                &swap.dst_amount,
                &swap.dst_safety_deposit,
                &swap.dst_timelocks,
                &swap.dst_status,
                &now,
                &now,
            ],
        ).await?;

        Ok(result > 0)
    }

    /// Update swap with destination data
    pub async fn update_fusion_plus_dst(
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
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = client.execute(
            "UPDATE fusion_plus_swaps SET
                dst_tx_hash = $1,
                dst_block_number = $2,
                dst_block_timestamp = $3,
                dst_log_index = $4,
                dst_escrow_address = $5,
                dst_taker = $6,
                dst_timelocks = $7,
                dst_status = 'created',
                updated_at = $8
             WHERE order_hash = $9 AND dst_chain_id = $10",
            &[
                &tx_hash.to_lowercase(),
                &(block_number as i64),
                &(block_timestamp as i64),
                &(log_index as i32),
                &escrow_address.map(|s| s.to_lowercase()),
                &dst_data.dst_taker.to_lowercase(),
                &dst_data.dst_timelocks,
                &now,
                &order_hash.to_lowercase(),
                &(chain_id as i32),
            ],
        ).await?;

        Ok(result > 0)
    }

    /// Update swap status on withdrawal
    pub async fn update_fusion_plus_withdrawal(
        &self,
        order_hash: &str,
        chain_id: u32,
        is_src: bool,
        secret: &str,
    ) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = if is_src {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    src_status = 'withdrawn',
                    secret = $1,
                    updated_at = $2
                 WHERE order_hash = $3 AND src_chain_id = $4",
                &[
                    &secret.to_lowercase(),
                    &now,
                    &order_hash.to_lowercase(),
                    &(chain_id as i32),
                ],
            ).await?
        } else {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    dst_status = 'withdrawn',
                    secret = $1,
                    updated_at = $2
                 WHERE order_hash = $3 AND dst_chain_id = $4",
                &[
                    &secret.to_lowercase(),
                    &now,
                    &order_hash.to_lowercase(),
                    &(chain_id as i32),
                ],
            ).await?
        };

        Ok(result > 0)
    }

    /// Update swap status on cancellation
    pub async fn update_fusion_plus_cancelled(
        &self,
        order_hash: &str,
        chain_id: u32,
        is_src: bool,
    ) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = if is_src {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    src_status = 'cancelled',
                    updated_at = $1
                 WHERE order_hash = $2 AND src_chain_id = $3",
                &[&now, &order_hash.to_lowercase(), &(chain_id as i32)],
            ).await?
        } else {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    dst_status = 'cancelled',
                    updated_at = $1
                 WHERE order_hash = $2 AND dst_chain_id = $3",
                &[&now, &order_hash.to_lowercase(), &(chain_id as i32)],
            ).await?
        };

        Ok(result > 0)
    }

    /// Update swap status on withdrawal by hashlock
    pub async fn update_fusion_plus_withdrawal_by_hashlock(
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
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = if is_src {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    src_status = 'withdrawn',
                    secret = $1,
                    updated_at = $2
                 WHERE hashlock = $3 AND src_chain_id = $4",
                &[
                    &secret.to_lowercase(),
                    &now,
                    &hashlock.to_lowercase(),
                    &(chain_id as i32),
                ],
            ).await?
        } else {
            client.execute(
                "UPDATE fusion_plus_swaps SET
                    dst_status = 'withdrawn',
                    dst_tx_hash = $5,
                    dst_block_number = $6,
                    dst_block_timestamp = $7,
                    dst_log_index = $8,
                    secret = $1,
                    updated_at = $2
                 WHERE hashlock = $3 AND dst_chain_id = $4",
                &[
                    &secret.to_lowercase(),
                    &now,
                    &hashlock.to_lowercase(),
                    &(chain_id as i32),
                    &tx_hash.to_lowercase(),
                    &(block_number as i64),
                    &(block_timestamp as i64),
                    &(log_index as i32),
                ],
            ).await?
        };

        Ok(result > 0)
    }

    fn row_to_fusion_plus_swap(row: &Row) -> FusionPlusSwap {
        FusionPlusSwap {
            order_hash: row.get(0),
            hashlock: row.get(1),
            secret: row.get(2),
            src_chain_id: row.get::<_, i32>(3) as u32,
            src_tx_hash: row.get(4),
            src_block_number: row.get::<_, i64>(5) as u64,
            src_block_timestamp: row.get::<_, i64>(6) as u64,
            src_log_index: row.get::<_, i32>(7) as u32,
            src_escrow_address: row.get(8),
            src_maker: row.get(9),
            src_taker: row.get(10),
            src_token: row.get(11),
            src_amount: row.get(12),
            src_safety_deposit: row.get(13),
            src_timelocks: row.get(14),
            src_status: row.get(15),
            dst_chain_id: row.get::<_, i32>(16) as u32,
            dst_tx_hash: row.get(17),
            dst_block_number: row.get::<_, Option<i64>>(18).map(|n| n as u64),
            dst_block_timestamp: row.get::<_, Option<i64>>(19).map(|n| n as u64),
            dst_log_index: row.get::<_, Option<i32>>(20).map(|n| n as u32),
            dst_escrow_address: row.get(21),
            dst_maker: row.get(22),
            dst_taker: row.get(23),
            dst_token: row.get(24),
            dst_amount: row.get(25),
            dst_safety_deposit: row.get(26),
            dst_timelocks: row.get(27),
            dst_status: row.get(28),
        }
    }

    /// Get Fusion+ swap by order_hash
    pub async fn get_fusion_plus_swap(&self, order_hash: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT order_hash, hashlock, secret,
                    src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                    src_escrow_address, src_maker, src_taker, src_token, src_amount,
                    src_safety_deposit, src_timelocks, src_status,
                    dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                    dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                    dst_safety_deposit, dst_timelocks, dst_status
             FROM fusion_plus_swaps WHERE order_hash = $1",
            &[&order_hash.to_lowercase()],
        ).await?;

        Ok(row.map(|r| Self::row_to_fusion_plus_swap(&r)))
    }

    /// Get Fusion+ swap by hashlock
    pub async fn get_fusion_plus_swap_by_hashlock(&self, hashlock: &str) -> Result<Option<FusionPlusSwap>, DbError> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT order_hash, hashlock, secret,
                    src_chain_id, src_tx_hash, src_block_number, src_block_timestamp, src_log_index,
                    src_escrow_address, src_maker, src_taker, src_token, src_amount,
                    src_safety_deposit, src_timelocks, src_status,
                    dst_chain_id, dst_tx_hash, dst_block_number, dst_block_timestamp, dst_log_index,
                    dst_escrow_address, dst_maker, dst_taker, dst_token, dst_amount,
                    dst_safety_deposit, dst_timelocks, dst_status
             FROM fusion_plus_swaps WHERE hashlock = $1",
            &[&hashlock.to_lowercase()],
        ).await?;

        Ok(row.map(|r| Self::row_to_fusion_plus_swap(&r)))
    }

    /// Get total count of Fusion+ swaps
    pub async fn get_fusion_plus_count(&self) -> Result<u64, DbError> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "SELECT COUNT(*) FROM fusion_plus_swaps",
            &[],
        ).await?;

        Ok(row.get::<_, i64>(0) as u64)
    }

    /// Clean up old Fusion+ swaps based on TTL
    pub async fn cleanup_old_fusion_plus(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - ttl_secs as i64;

        let deleted = client.execute(
            "DELETE FROM fusion_plus_swaps WHERE created_at < $1",
            &[&cutoff],
        ).await?;

        Ok(deleted as usize)
    }

    // =========================================================================
    // Fusion (Single-Chain) Methods
    // =========================================================================

    /// Insert a new Fusion swap
    pub async fn insert_fusion_swap(&self, swap: &FusionSwap) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = client.execute(
            "INSERT INTO fusion_swaps (
                order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                remaining, is_partial_fill, status, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
            &[
                &swap.order_hash.to_lowercase(),
                &(swap.chain_id as i32),
                &swap.tx_hash.to_lowercase(),
                &(swap.block_number as i64),
                &(swap.block_timestamp as i64),
                &(swap.log_index as i32),
                &swap.maker.to_lowercase(),
                &swap.taker.as_ref().map(|s| s.to_lowercase()),
                &swap.maker_token.as_ref().map(|s| s.to_lowercase()),
                &swap.taker_token.as_ref().map(|s| s.to_lowercase()),
                &swap.maker_amount,
                &swap.taker_amount,
                &swap.remaining,
                &swap.is_partial_fill,
                &swap.status,
                &now,
            ],
        ).await?;

        Ok(result > 0)
    }

    fn row_to_fusion_swap(row: &Row) -> FusionSwap {
        FusionSwap {
            order_hash: row.get(0),
            chain_id: row.get::<_, i32>(1) as u32,
            tx_hash: row.get(2),
            block_number: row.get::<_, i64>(3) as u64,
            block_timestamp: row.get::<_, i64>(4) as u64,
            log_index: row.get::<_, i32>(5) as u32,
            maker: row.get(6),
            taker: row.get(7),
            maker_token: row.get(8),
            taker_token: row.get(9),
            maker_amount: row.get(10),
            taker_amount: row.get(11),
            remaining: row.get(12),
            is_partial_fill: row.get(13),
            status: row.get(14),
        }
    }

    /// Get Fusion swap by order_hash
    pub async fn get_fusion_swap_by_order_hash(&self, order_hash: &str) -> Result<Option<FusionSwap>, DbError> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT order_hash, chain_id, tx_hash, block_number, block_timestamp, log_index,
                    maker, taker, maker_token, taker_token, maker_amount, taker_amount,
                    remaining, is_partial_fill, status
             FROM fusion_swaps WHERE order_hash = $1
             ORDER BY block_timestamp DESC LIMIT 1",
            &[&order_hash.to_lowercase()],
        ).await?;

        Ok(row.map(|r| Self::row_to_fusion_swap(&r)))
    }

    /// Get total count of Fusion swaps
    pub async fn get_fusion_swap_count(&self) -> Result<u64, DbError> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "SELECT COUNT(*) FROM fusion_swaps",
            &[],
        ).await?;

        Ok(row.get::<_, i64>(0) as u64)
    }

    /// Clean up old Fusion swaps based on TTL
    pub async fn cleanup_old_fusion_swaps(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - ttl_secs as i64;

        let deleted = client.execute(
            "DELETE FROM fusion_swaps WHERE created_at < $1",
            &[&cutoff],
        ).await?;

        Ok(deleted as usize)
    }

    // =========================================================================
    // Crypto2Fiat Methods
    // =========================================================================

    /// Insert a new Crypto2Fiat event
    pub async fn insert_crypto2fiat_event(&self, event: &Crypto2FiatEvent) -> Result<bool, DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = client.execute(
            "INSERT INTO crypto2fiat_events (
                order_id, token, amount, recipient, metadata,
                chain_id, tx_hash, block_number, block_timestamp, log_index, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
            &[
                &event.order_id.to_lowercase(),
                &event.token.to_lowercase(),
                &event.amount,
                &event.recipient.to_lowercase(),
                &event.metadata,
                &(event.chain_id as i32),
                &event.tx_hash.to_lowercase(),
                &(event.block_number as i64),
                &(event.block_timestamp as i64),
                &(event.log_index as i32),
                &now,
            ],
        ).await?;

        Ok(result > 0)
    }

    /// Get total count of Crypto2Fiat events
    pub async fn get_crypto2fiat_count(&self) -> Result<u64, DbError> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "SELECT COUNT(*) FROM crypto2fiat_events",
            &[],
        ).await?;

        Ok(row.get::<_, i64>(0) as u64)
    }

    /// Clean up old Crypto2Fiat events based on TTL
    pub async fn cleanup_old_crypto2fiat(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - ttl_secs as i64;

        let deleted = client.execute(
            "DELETE FROM crypto2fiat_events WHERE created_at < $1",
            &[&cutoff],
        ).await?;

        Ok(deleted as usize)
    }

    // =========================================================================
    // Cleanup Methods
    // =========================================================================

    /// Clean up all old data based on TTL
    pub async fn cleanup_all(&self, ttl_secs: u64) -> Result<CleanupStats, DbError> {
        let transfers = self.cleanup_old_transfers(ttl_secs).await?;
        let fusion_plus = self.cleanup_old_fusion_plus(ttl_secs).await?;
        let fusion = self.cleanup_old_fusion_swaps(ttl_secs).await?;
        let crypto2fiat = self.cleanup_old_crypto2fiat(ttl_secs).await?;

        Ok(CleanupStats {
            transfers_deleted: transfers,
            fusion_plus_deleted: fusion_plus,
            fusion_deleted: fusion,
            crypto2fiat_deleted: crypto2fiat,
        })
    }
}

#[derive(Default, Debug)]
pub struct CleanupStats {
    pub transfers_deleted: usize,
    pub fusion_plus_deleted: usize,
    pub fusion_deleted: usize,
    pub crypto2fiat_deleted: usize,
}
