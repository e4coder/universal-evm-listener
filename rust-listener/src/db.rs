use crate::types::{Crypto2FiatEvent, DstEscrowCreatedData, FusionPlusSwap, FusionSwap, Transfer};
use deadpool_postgres::{Config, Pool, Runtime, PoolError, Object as PoolClient};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio_postgres::{NoTls, Row, types::ToSql};
use bytes::{BytesMut, BufMut};
use futures::SinkExt;
use tracing::info;

/// Normalize address/hash: lowercase only for EVM hex strings (0x-prefixed).
/// Bitcoin/Solana base58 addresses are case-sensitive and must not be lowercased.
#[inline]
fn normalize(s: &str) -> String {
    if s.starts_with("0x") || s.starts_with("0X") {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

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
    pub async fn new(database_url: &str, chain_ids: &[u32]) -> Result<Self, DbError> {
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

        // Limit pool size to prevent unbounded connection growth
        // 13 chains × 3 concurrent inserts = 39 + cleanup + stats + config watcher + headroom ≈ 50
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: 50,
            ..Default::default()
        });

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| DbError::Config(e.to_string()))?;

        let db = Self { pool };

        // Auto-create schema on startup
        db.create_schema(chain_ids).await?;

        Ok(db)
    }

    /// Acquire a pooled connection (for callers that manage their own transaction)
    pub async fn get_client(&self) -> Result<PoolClient, DbError> {
        Ok(self.pool.get().await?)
    }

    /// Create all tables and indexes if they don't exist
    async fn create_schema(&self, chain_ids: &[u32]) -> Result<(), DbError> {
        let client = self.pool.get().await?;

        // Migrate non-partitioned transfers table to partitioned (10-min TTL — brief data gap OK)
        let table_exists = client.query_opt(
            "SELECT 1 FROM pg_class WHERE relname = 'transfers' AND relnamespace = 'public'::regnamespace",
            &[],
        ).await?.is_some();

        if table_exists {
            let is_partitioned = client.query_opt(
                "SELECT 1 FROM pg_class WHERE relname = 'transfers' AND relkind = 'p'",
                &[],
            ).await?.is_some();

            if !is_partitioned {
                info!("Migrating transfers table to partitioned schema (LIST by chain_id)...");
                client.execute("DROP TABLE transfers CASCADE", &[]).await?;
                info!("Old non-partitioned transfers table dropped");
            }
        }

        // Transfers table — partitioned by chain_id for per-chain index locality
        client.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                id BIGSERIAL,
                chain_id INTEGER NOT NULL,
                tx_hash VARCHAR(128) NOT NULL,
                log_index INTEGER NOT NULL,
                token VARCHAR(128) NOT NULL,
                from_addr VARCHAR(128) NOT NULL,
                to_addr VARCHAR(128) NOT NULL,
                value VARCHAR(78) NOT NULL,
                block_number BIGINT NOT NULL,
                block_timestamp BIGINT NOT NULL,
                swap_type VARCHAR(20),
                status VARCHAR(10),
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                PRIMARY KEY (chain_id, id),
                UNIQUE(chain_id, tx_hash, log_index)
            ) PARTITION BY LIST (chain_id)",
            &[],
        ).await?;

        // Create partitions concurrently (non-blocking — each is IF NOT EXISTS)
        let mut partition_futures = Vec::new();
        for &chain_id in chain_ids {
            let pool = self.pool.clone();
            partition_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute(
                        &format!(
                            "CREATE TABLE IF NOT EXISTS transfers_{} PARTITION OF transfers FOR VALUES IN ({})",
                            chain_id, chain_id
                        ),
                        &[],
                    ).await.ok();
                }
            }));
        }
        // Wait for all partitions (they run in parallel)
        for f in partition_futures {
            f.await.ok();
        }

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

        // Listener stats table (monitoring dashboard)
        client.execute(
            "CREATE TABLE IF NOT EXISTS listener_stats (
                chain_id INTEGER PRIMARY KEY,
                chain_name VARCHAR(30) NOT NULL,
                current_block BIGINT DEFAULT 0,
                checkpoint_block BIGINT DEFAULT 0,
                pending_ranges INTEGER DEFAULT 0,
                last_chance_count INTEGER DEFAULT 0,
                inflight_fetches INTEGER DEFAULT 0,
                successful_fetches BIGINT DEFAULT 0,
                failed_fetches BIGINT DEFAULT 0,
                timed_out_fetches BIGINT DEFAULT 0,
                blocks_processed BIGINT DEFAULT 0,
                total_transfers BIGINT DEFAULT 0,
                buffer_size INTEGER DEFAULT 0,
                insert_time_ms INTEGER DEFAULT 0,
                batch_size INTEGER DEFAULT 0,
                updated_at BIGINT DEFAULT 0
            )",
            &[],
        ).await?;

        // Historical metrics table (time-series for monitoring charts)
        client.execute(
            "CREATE TABLE IF NOT EXISTS listener_metrics_history (
                id BIGSERIAL PRIMARY KEY,
                chain_id INTEGER NOT NULL,
                recorded_at BIGINT NOT NULL,
                insert_time_ms INTEGER DEFAULT 0,
                batch_size INTEGER DEFAULT 0,
                buffer_size INTEGER DEFAULT 0,
                blocks_behind BIGINT DEFAULT 0,
                events_total BIGINT DEFAULT 0,
                fetch_time_ms INTEGER DEFAULT 0
            )",
            &[],
        ).await?;

        // Config overrides table (runtime-tunable per-chain config)
        client.execute(
            "CREATE TABLE IF NOT EXISTS config_overrides (
                chain_id INTEGER PRIMARY KEY,
                blocks_per_request INTEGER,
                concurrent_fetches INTEGER,
                poll_interval_ms BIGINT,
                confirmation_blocks INTEGER,
                copy_threshold INTEGER,
                updated_at BIGINT NOT NULL
            )",
            &[],
        ).await?;

        // Run all ADD COLUMN migrations + column widening check in parallel
        // Each uses its own pool connection since they touch different tables
        let pool_m = self.pool.clone();
        let mut migration_futures: Vec<tokio::task::JoinHandle<()>> = Vec::new();

        // listener_stats columns
        {
            let pool = pool_m.clone();
            migration_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute("ALTER TABLE listener_stats ADD COLUMN IF NOT EXISTS insert_time_ms INTEGER DEFAULT 0", &[]).await.ok();
                    c.execute("ALTER TABLE listener_stats ADD COLUMN IF NOT EXISTS batch_size INTEGER DEFAULT 0", &[]).await.ok();
                    c.execute("ALTER TABLE listener_stats ADD COLUMN IF NOT EXISTS fetch_time_ms INTEGER DEFAULT 0", &[]).await.ok();
                }
            }));
        }
        // listener_metrics_history columns
        {
            let pool = pool_m.clone();
            migration_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute("ALTER TABLE listener_metrics_history ADD COLUMN IF NOT EXISTS fetch_time_ms INTEGER DEFAULT 0", &[]).await.ok();
                }
            }));
        }
        // config_overrides columns
        {
            let pool = pool_m.clone();
            migration_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute("ALTER TABLE config_overrides ADD COLUMN IF NOT EXISTS concurrent_inserts INTEGER", &[]).await.ok();
                }
            }));
        }
        // transfers status column + column widening
        {
            let pool = pool_m.clone();
            migration_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute("ALTER TABLE transfers ADD COLUMN IF NOT EXISTS status VARCHAR(10)", &[]).await.ok();
                    // Widen columns only if needed (ALTER COLUMN TYPE rewrites all partitions — skip if already correct)
                    let needs_widen: bool = c.query_opt(
                        "SELECT 1 FROM information_schema.columns
                         WHERE table_name = 'transfers' AND column_name = 'tx_hash'
                           AND character_maximum_length IS NOT NULL AND character_maximum_length < 128",
                        &[],
                    ).await.ok().flatten().is_some();
                    if needs_widen {
                        info!("Widening transfers columns to VARCHAR(128)...");
                        c.execute("ALTER TABLE transfers ALTER COLUMN from_addr TYPE VARCHAR(128)", &[]).await.ok();
                        c.execute("ALTER TABLE transfers ALTER COLUMN to_addr TYPE VARCHAR(128)", &[]).await.ok();
                        c.execute("ALTER TABLE transfers ALTER COLUMN token TYPE VARCHAR(128)", &[]).await.ok();
                        c.execute("ALTER TABLE transfers ALTER COLUMN tx_hash TYPE VARCHAR(128)", &[]).await.ok();
                        info!("Column widening complete");
                    }
                }
            }));
        }
        // Wait for all migrations to finish
        for f in migration_futures {
            f.await.ok();
        }

        // Drop redundant index + create all indexes concurrently
        // Each index creation gets its own pool connection for parallelism
        client.execute("DROP INDEX IF EXISTS idx_transfers_tx_hash", &[]).await?;

        let all_indexes: Vec<&str> = vec![
            // transfers indexes
            "CREATE INDEX IF NOT EXISTS idx_transfers_from ON transfers(chain_id, from_addr, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_to ON transfers(chain_id, to_addr, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_created ON transfers(created_at)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_swap_type ON transfers(chain_id, swap_type, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_from_id ON transfers(chain_id, from_addr, id)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_to_id ON transfers(chain_id, to_addr, id)",
            "CREATE INDEX IF NOT EXISTS idx_transfers_status ON transfers(chain_id, status) WHERE status IS NOT NULL",
            // fusion_plus_swaps indexes
            "CREATE INDEX IF NOT EXISTS idx_fp_hashlock ON fusion_plus_swaps(hashlock)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_chain ON fusion_plus_swaps(src_chain_id, src_block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_chain ON fusion_plus_swaps(dst_chain_id, dst_block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_maker ON fusion_plus_swaps(src_maker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_dst_maker ON fusion_plus_swaps(dst_maker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_src_taker ON fusion_plus_swaps(src_taker)",
            "CREATE INDEX IF NOT EXISTS idx_fp_status ON fusion_plus_swaps(src_status, dst_status)",
            "CREATE INDEX IF NOT EXISTS idx_fp_created ON fusion_plus_swaps(created_at)",
            // fusion_swaps indexes
            "CREATE INDEX IF NOT EXISTS idx_fs_order_hash ON fusion_swaps(order_hash)",
            "CREATE INDEX IF NOT EXISTS idx_fs_chain ON fusion_swaps(chain_id, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_fs_maker ON fusion_swaps(maker)",
            "CREATE INDEX IF NOT EXISTS idx_fs_taker ON fusion_swaps(taker)",
            "CREATE INDEX IF NOT EXISTS idx_fs_status ON fusion_swaps(status)",
            "CREATE INDEX IF NOT EXISTS idx_fs_created ON fusion_swaps(created_at)",
            // crypto2fiat_events indexes
            "CREATE INDEX IF NOT EXISTS idx_c2f_order_id ON crypto2fiat_events(order_id)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_token ON crypto2fiat_events(token)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_recipient ON crypto2fiat_events(recipient)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_chain ON crypto2fiat_events(chain_id, block_timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_c2f_created ON crypto2fiat_events(created_at)",
            // metrics history index
            "CREATE INDEX IF NOT EXISTS idx_metrics_chain_time ON listener_metrics_history(chain_id, recorded_at DESC)",
        ];

        let mut index_futures = Vec::new();
        for sql in all_indexes {
            let pool = self.pool.clone();
            index_futures.push(tokio::spawn(async move {
                if let Ok(c) = pool.get().await {
                    c.execute(sql, &[]).await.ok();
                }
            }));
        }
        for f in index_futures {
            f.await.ok();
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
             (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
                &transfer.status,
                &now,
            ],
        ).await?;

        Ok(result > 0)
    }

    /// Insert transfers using COPY protocol via staging table for high throughput.
    /// Uses: CREATE TEMP TABLE → COPY FROM STDIN → INSERT...SELECT ON CONFLICT DO NOTHING
    async fn insert_transfers_copy(&self, chain_id: u32, transfers: &[Transfer]) -> Result<usize, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let chain_id_i32 = chain_id as i32;
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        // 1. Create temp staging table (auto-drops on commit/rollback)
        tx.execute(
            "CREATE TEMP TABLE _transfers_staging (LIKE transfers INCLUDING DEFAULTS) ON COMMIT DROP",
            &[],
        ).await?;

        // 2. COPY data into staging table via text-format stream
        let sink = tx.copy_in(
            "COPY _transfers_staging (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) FROM STDIN"
        ).await?;
        futures::pin_mut!(sink);

        // Build tab-separated rows, flush every ~64KB
        let mut buf = BytesMut::with_capacity(64 * 1024);
        for t in transfers {
            // All fields are integers, hex strings, or known enums — no escaping needed
            buf.extend_from_slice(chain_id_i32.to_string().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice(t.tx_hash.to_lowercase().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice((t.log_index as i32).to_string().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice(t.token.to_lowercase().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice(t.from_addr.to_lowercase().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice(t.to_addr.to_lowercase().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice(t.value.as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice((t.block_number as i64).to_string().as_bytes());
            buf.put_u8(b'\t');
            buf.extend_from_slice((t.block_timestamp as i64).to_string().as_bytes());
            buf.put_u8(b'\t');
            match &t.swap_type {
                Some(s) => buf.extend_from_slice(s.as_bytes()),
                None => buf.extend_from_slice(b"\\N"),
            }
            buf.put_u8(b'\t');
            match &t.status {
                Some(s) => buf.extend_from_slice(s.as_bytes()),
                None => buf.extend_from_slice(b"\\N"),
            }
            buf.put_u8(b'\t');
            buf.extend_from_slice(now.to_string().as_bytes());
            buf.put_u8(b'\n');

            if buf.len() >= 64 * 1024 {
                sink.send(buf.split().freeze()).await.map_err(DbError::Postgres)?;
            }
        }
        if !buf.is_empty() {
            sink.send(buf.freeze()).await.map_err(DbError::Postgres)?;
        }
        sink.finish().await?;

        // 3. Move from staging → real table with ON CONFLICT
        let result = tx.execute(
            "INSERT INTO transfers (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) \
             SELECT chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at \
             FROM _transfers_staging \
             ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
            &[],
        ).await?;

        tx.commit().await?;
        Ok(result as usize)
    }

    /// Insert multiple transfers — dispatches to COPY (fast) or multi-row INSERT (fallback)
    pub async fn insert_transfers_batch(&self, chain_id: u32, transfers: &[Transfer], copy_threshold: usize) -> Result<usize, DbError> {
        if transfers.is_empty() {
            return Ok(0);
        }

        // Use COPY protocol for batches >= threshold (runtime-configurable via admin API)
        if transfers.len() >= copy_threshold {
            return self.insert_transfers_copy(chain_id, transfers).await;
        }

        // Fallback: multi-row INSERT for small batches
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let chain_id_i32 = chain_id as i32;
        let mut total_inserted = 0;

        for chunk in transfers.chunks(1500) {
            let rows: Vec<(i32, String, i32, String, String, String, String, i64, i64, Option<String>, Option<String>, i64)> =
                chunk.iter().map(|t| (
                    chain_id_i32,
                    t.tx_hash.to_lowercase(),
                    t.log_index as i32,
                    t.token.to_lowercase(),
                    t.from_addr.to_lowercase(),
                    t.to_addr.to_lowercase(),
                    t.value.clone(),
                    t.block_number as i64,
                    t.block_timestamp as i64,
                    t.swap_type.clone(),
                    t.status.clone(),
                    now,
                )).collect();

            let mut values_parts = Vec::with_capacity(chunk.len());
            let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(chunk.len() * 12);

            for (i, row) in rows.iter().enumerate() {
                let b = i * 12;
                values_parts.push(format!(
                    "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                    b+1, b+2, b+3, b+4, b+5, b+6, b+7, b+8, b+9, b+10, b+11, b+12
                ));
                params.push(&row.0);
                params.push(&row.1);
                params.push(&row.2);
                params.push(&row.3);
                params.push(&row.4);
                params.push(&row.5);
                params.push(&row.6);
                params.push(&row.7);
                params.push(&row.8);
                params.push(&row.9);
                params.push(&row.10);
                params.push(&row.11);
            }

            let sql = format!(
                "INSERT INTO transfers \
                 (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) \
                 VALUES {} \
                 ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
                values_parts.join(", ")
            );

            let result = client.execute(sql.as_str(), &params).await?;
            total_inserted += result as usize;
        }

        Ok(total_inserted)
    }

    // =========================================================================
    // Transaction-aware methods (_on variants)
    // These run on an externally-provided transaction instead of acquiring
    // their own pool connection. Used by process_fetched_data() for atomicity.
    // =========================================================================

    /// Insert transfers within an existing transaction (COPY or multi-row INSERT)
    pub(crate) async fn insert_transfers_batch_on(
        tx: &tokio_postgres::Transaction<'_>,
        chain_id: u32,
        transfers: &[Transfer],
        copy_threshold: usize,
    ) -> Result<usize, DbError> {
        if transfers.is_empty() {
            return Ok(0);
        }

        if transfers.len() >= copy_threshold {
            return Self::insert_transfers_copy_on(tx, chain_id, transfers).await;
        }

        // Multi-row INSERT path (same logic as insert_transfers_batch)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let chain_id_i32 = chain_id as i32;
        let mut total_inserted = 0;

        for chunk in transfers.chunks(1500) {
            let rows: Vec<(i32, String, i32, String, String, String, String, i64, i64, Option<String>, Option<String>, i64)> =
                chunk.iter().map(|t| (
                    chain_id_i32,
                    normalize(&t.tx_hash),
                    t.log_index as i32,
                    normalize(&t.token),
                    normalize(&t.from_addr),
                    normalize(&t.to_addr),
                    t.value.clone(),
                    t.block_number as i64,
                    t.block_timestamp as i64,
                    t.swap_type.clone(),
                    t.status.clone(),
                    now,
                )).collect();

            let mut values_parts = Vec::with_capacity(chunk.len());
            let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(chunk.len() * 12);

            for (i, row) in rows.iter().enumerate() {
                let b = i * 12;
                values_parts.push(format!(
                    "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                    b+1, b+2, b+3, b+4, b+5, b+6, b+7, b+8, b+9, b+10, b+11, b+12
                ));
                params.push(&row.0);
                params.push(&row.1);
                params.push(&row.2);
                params.push(&row.3);
                params.push(&row.4);
                params.push(&row.5);
                params.push(&row.6);
                params.push(&row.7);
                params.push(&row.8);
                params.push(&row.9);
                params.push(&row.10);
                params.push(&row.11);
            }

            let sql = format!(
                "INSERT INTO transfers \
                 (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) \
                 VALUES {} \
                 ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
                values_parts.join(", ")
            );

            let result = tx.execute(sql.as_str(), &params).await?;
            total_inserted += result as usize;
        }

        Ok(total_inserted)
    }

    /// COPY-based transfer insert within an existing transaction.
    /// Chunks large batches into ~1000-row sub-batches to reduce index maintenance
    /// pressure and buffer pool thrashing per INSERT...SELECT ON CONFLICT.
    async fn insert_transfers_copy_on(
        tx: &tokio_postgres::Transaction<'_>,
        chain_id: u32,
        transfers: &[Transfer],
    ) -> Result<usize, DbError> {
        const INSERT_CHUNK: usize = 1000;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let chain_id_i32 = chain_id as i32;

        // Create temp staging table once (ON COMMIT DROP fires when outer tx commits)
        tx.execute(
            "CREATE TEMP TABLE _transfers_staging (LIKE transfers INCLUDING DEFAULTS) ON COMMIT DROP",
            &[],
        ).await?;

        let mut total_inserted = 0usize;

        for chunk in transfers.chunks(INSERT_CHUNK) {
            // COPY chunk into staging table
            let sink = tx.copy_in(
                "COPY _transfers_staging (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) FROM STDIN"
            ).await?;
            futures::pin_mut!(sink);

            let mut buf = BytesMut::with_capacity(64 * 1024);
            for t in chunk {
                buf.extend_from_slice(chain_id_i32.to_string().as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice(normalize(&t.tx_hash).as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice((t.log_index as i32).to_string().as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice(normalize(&t.token).as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice(normalize(&t.from_addr).as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice(normalize(&t.to_addr).as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice(t.value.as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice((t.block_number as i64).to_string().as_bytes());
                buf.put_u8(b'\t');
                buf.extend_from_slice((t.block_timestamp as i64).to_string().as_bytes());
                buf.put_u8(b'\t');
                match &t.swap_type {
                    Some(s) => buf.extend_from_slice(s.as_bytes()),
                    None => buf.extend_from_slice(b"\\N"),
                }
                buf.put_u8(b'\t');
                match &t.status {
                    Some(s) => buf.extend_from_slice(s.as_bytes()),
                    None => buf.extend_from_slice(b"\\N"),
                }
                buf.put_u8(b'\t');
                buf.extend_from_slice(now.to_string().as_bytes());
                buf.put_u8(b'\n');

                if buf.len() >= 64 * 1024 {
                    sink.send(buf.split().freeze()).await.map_err(DbError::Postgres)?;
                }
            }
            if !buf.is_empty() {
                sink.send(buf.freeze()).await.map_err(DbError::Postgres)?;
            }
            sink.finish().await?;

            // Move from staging → real table with ON CONFLICT
            let result = tx.execute(
                "INSERT INTO transfers (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at) \
                 SELECT chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at \
                 FROM _transfers_staging \
                 ON CONFLICT (chain_id, tx_hash, log_index) DO NOTHING",
                &[],
            ).await?;

            total_inserted += result as usize;

            // Clear staging for next chunk
            tx.execute("TRUNCATE _transfers_staging", &[]).await?;
        }

        Ok(total_inserted)
    }

    /// Get first and last transfers within an existing transaction
    pub(crate) async fn get_first_last_transfers_on(
        tx: &tokio_postgres::Transaction<'_>,
        chain_id: u32,
        tx_hash: &str,
    ) -> Result<Option<(Transfer, Transfer)>, DbError> {
        let tx_hash_lower = tx_hash.to_lowercase();

        let first_row = tx.query_opt(
            "SELECT tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type
             FROM transfers
             WHERE chain_id = $1 AND tx_hash = $2
             ORDER BY log_index ASC
             LIMIT 1",
            &[&(chain_id as i32), &tx_hash_lower],
        ).await?;

        let last_row = tx.query_opt(
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
                    status: None,
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
                    status: None,
                };
                Ok(Some((first_transfer, last_transfer)))
            }
            _ => Ok(None),
        }
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
                    status: None,
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
                    status: None,
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

    // --- Fusion+ transaction-aware methods ---

    /// Insert a Fusion+ swap within an existing transaction
    pub(crate) async fn insert_fusion_plus_swap_on(
        tx: &tokio_postgres::Transaction<'_>,
        swap: &FusionPlusSwap,
    ) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = tx.execute(
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

    /// Update Fusion+ swap with destination data within an existing transaction
    pub(crate) async fn update_fusion_plus_dst_on(
        tx: &tokio_postgres::Transaction<'_>,
        order_hash: &str,
        dst_data: &DstEscrowCreatedData,
        chain_id: u32,
        tx_hash_str: &str,
        block_number: u64,
        block_timestamp: u64,
        log_index: u32,
        escrow_address: Option<&str>,
    ) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = tx.execute(
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
                &tx_hash_str.to_lowercase(),
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

    /// Update Fusion+ withdrawal by hashlock within an existing transaction
    pub(crate) async fn update_fusion_plus_withdrawal_by_hashlock_on(
        tx: &tokio_postgres::Transaction<'_>,
        hashlock: &str,
        chain_id: u32,
        is_src: bool,
        secret: &str,
        tx_hash_str: &str,
        block_number: u64,
        block_timestamp: u64,
        log_index: u32,
    ) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = if is_src {
            tx.execute(
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
            tx.execute(
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
                    &tx_hash_str.to_lowercase(),
                    &(block_number as i64),
                    &(block_timestamp as i64),
                    &(log_index as i32),
                ],
            ).await?
        };

        Ok(result > 0)
    }

    /// Get Fusion+ swap by hashlock within an existing transaction
    pub(crate) async fn get_fusion_plus_swap_by_hashlock_on(
        tx: &tokio_postgres::Transaction<'_>,
        hashlock: &str,
    ) -> Result<Option<FusionPlusSwap>, DbError> {
        let row = tx.query_opt(
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

    // --- Fusion single-chain transaction-aware methods ---

    /// Insert a Fusion swap within an existing transaction
    pub(crate) async fn insert_fusion_swap_on(
        tx: &tokio_postgres::Transaction<'_>,
        swap: &FusionSwap,
    ) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = tx.execute(
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

    // --- Crypto2Fiat transaction-aware methods ---

    /// Insert a Crypto2Fiat event within an existing transaction
    pub(crate) async fn insert_crypto2fiat_event_on(
        tx: &tokio_postgres::Transaction<'_>,
        event: &Crypto2FiatEvent,
    ) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = tx.execute(
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

    // =========================================================================
    // Listener Stats Methods (Monitoring)
    // =========================================================================

    /// Upsert listener stats for a single chain
    pub async fn upsert_listener_stats(
        &self,
        chain_id: u32,
        chain_name: &str,
        current_block: u64,
        checkpoint_block: u64,
        pending_ranges: u64,
        last_chance_count: u64,
        inflight_fetches: u64,
        successful_fetches: u64,
        failed_fetches: u64,
        timed_out_fetches: u64,
        blocks_processed: u64,
        total_transfers: u64,
        buffer_size: u64,
        insert_time_ms: u64,
        batch_size: u64,
        fetch_time_ms: u64,
    ) -> Result<(), DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        client.execute(
            "INSERT INTO listener_stats (
                chain_id, chain_name, current_block, checkpoint_block,
                pending_ranges, last_chance_count, inflight_fetches,
                successful_fetches, failed_fetches, timed_out_fetches,
                blocks_processed, total_transfers, buffer_size,
                insert_time_ms, batch_size, fetch_time_ms, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT (chain_id) DO UPDATE SET
                chain_name = EXCLUDED.chain_name,
                current_block = EXCLUDED.current_block,
                checkpoint_block = EXCLUDED.checkpoint_block,
                pending_ranges = EXCLUDED.pending_ranges,
                last_chance_count = EXCLUDED.last_chance_count,
                inflight_fetches = EXCLUDED.inflight_fetches,
                successful_fetches = EXCLUDED.successful_fetches,
                failed_fetches = EXCLUDED.failed_fetches,
                timed_out_fetches = EXCLUDED.timed_out_fetches,
                blocks_processed = EXCLUDED.blocks_processed,
                total_transfers = EXCLUDED.total_transfers,
                buffer_size = EXCLUDED.buffer_size,
                insert_time_ms = EXCLUDED.insert_time_ms,
                batch_size = EXCLUDED.batch_size,
                fetch_time_ms = EXCLUDED.fetch_time_ms,
                updated_at = EXCLUDED.updated_at",
            &[
                &(chain_id as i32),
                &chain_name,
                &(current_block as i64),
                &(checkpoint_block as i64),
                &(pending_ranges as i32),
                &(last_chance_count as i32),
                &(inflight_fetches as i32),
                &(successful_fetches as i64),
                &(failed_fetches as i64),
                &(timed_out_fetches as i64),
                &(blocks_processed as i64),
                &(total_transfers as i64),
                &(buffer_size as i32),
                &(insert_time_ms as i32),
                &(batch_size as i32),
                &(fetch_time_ms as i32),
                &now,
            ],
        ).await?;

        Ok(())
    }

    /// Insert a historical metrics snapshot for a chain
    pub async fn insert_metrics_snapshot(
        &self,
        chain_id: u32,
        insert_time_ms: u64,
        batch_size: u64,
        buffer_size: u64,
        blocks_behind: u64,
        events_total: u64,
        fetch_time_ms: u64,
    ) -> Result<(), DbError> {
        let client = self.pool.get().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        client.execute(
            "INSERT INTO listener_metrics_history \
             (chain_id, recorded_at, insert_time_ms, batch_size, buffer_size, blocks_behind, events_total, fetch_time_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &(chain_id as i32),
                &now,
                &(insert_time_ms as i32),
                &(batch_size as i32),
                &(buffer_size as i32),
                &(blocks_behind as i64),
                &(events_total as i64),
                &(fetch_time_ms as i32),
            ],
        ).await?;

        Ok(())
    }

    // =========================================================================
    // Cleanup Methods
    // =========================================================================

    /// Clean up old metrics history based on TTL
    // =========================================================================
    // Config Overrides CRUD
    // =========================================================================

    /// Read all config overrides (for config watcher)
    pub async fn get_all_config_overrides(&self) -> Result<Vec<ConfigOverrideRow>, DbError> {
        let client = self.pool.get().await?;
        let rows = client.query(
            "SELECT chain_id, blocks_per_request, concurrent_fetches, poll_interval_ms, confirmation_blocks, copy_threshold, concurrent_inserts FROM config_overrides",
            &[],
        ).await?;

        Ok(rows.iter().map(|row| ConfigOverrideRow {
            chain_id: row.get(0),
            blocks_per_request: row.get(1),
            concurrent_fetches: row.get(2),
            poll_interval_ms: row.get(3),
            confirmation_blocks: row.get(4),
            copy_threshold: row.get(5),
            concurrent_inserts: row.get(6),
        }).collect())
    }

    pub async fn cleanup_metrics_history(&self, ttl_secs: u64) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - ttl_secs as i64;

        let result = client.execute(
            "DELETE FROM listener_metrics_history WHERE recorded_at < $1",
            &[&cutoff],
        ).await?;
        Ok(result as usize)
    }

    /// Clean up all old data based on TTL
    pub async fn cleanup_all(&self, ttl_secs: u64) -> Result<CleanupStats, DbError> {
        let transfers = self.cleanup_old_transfers(ttl_secs).await?;
        let fusion_plus = self.cleanup_old_fusion_plus(ttl_secs).await?;
        let fusion = self.cleanup_old_fusion_swaps(ttl_secs).await?;
        let crypto2fiat = self.cleanup_old_crypto2fiat(ttl_secs).await?;
        let metrics = self.cleanup_metrics_history(ttl_secs).await?;

        Ok(CleanupStats {
            transfers_deleted: transfers,
            fusion_plus_deleted: fusion_plus,
            fusion_deleted: fusion,
            crypto2fiat_deleted: crypto2fiat,
            metrics_deleted: metrics,
        })
    }

    // =========================================================================
    // Bitcoin Mempool Transfer Methods
    // =========================================================================

    /// Upsert Bitcoin transfers (pending or confirmed).
    /// ON CONFLICT: updates status and block info when transitioning pending→confirmed.
    pub async fn upsert_bitcoin_transfers(
        &self,
        chain_id: u32,
        transfers: &[Transfer],
    ) -> Result<usize, DbError> {
        if transfers.is_empty() {
            return Ok(0);
        }
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        let count = Self::upsert_bitcoin_transfers_on(&tx, chain_id, transfers).await?;
        tx.commit().await?;
        Ok(count)
    }

    /// Upsert Bitcoin transfers within an existing transaction.
    pub(crate) async fn upsert_bitcoin_transfers_on(
        tx: &tokio_postgres::Transaction<'_>,
        chain_id: u32,
        transfers: &[Transfer],
    ) -> Result<usize, DbError> {
        if transfers.is_empty() {
            return Ok(0);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let chain_id_i32 = chain_id as i32;
        let mut total_upserted = 0;

        for chunk in transfers.chunks(500) {
            let rows: Vec<(i32, String, i32, String, String, String, String, i64, i64, Option<String>, Option<String>, i64)> =
                chunk.iter().map(|t| (
                    chain_id_i32,
                    t.tx_hash.clone(),
                    t.log_index as i32,
                    t.token.clone(),
                    t.from_addr.clone(),
                    t.to_addr.clone(),
                    t.value.clone(),
                    t.block_number as i64,
                    t.block_timestamp as i64,
                    t.swap_type.clone(),
                    t.status.clone(),
                    now,
                )).collect();

            let mut values_parts = Vec::with_capacity(chunk.len());
            let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(chunk.len() * 12);

            for (i, row) in rows.iter().enumerate() {
                let b = i * 12;
                values_parts.push(format!(
                    "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                    b+1, b+2, b+3, b+4, b+5, b+6, b+7, b+8, b+9, b+10, b+11, b+12
                ));
                params.push(&row.0);
                params.push(&row.1);
                params.push(&row.2);
                params.push(&row.3);
                params.push(&row.4);
                params.push(&row.5);
                params.push(&row.6);
                params.push(&row.7);
                params.push(&row.8);
                params.push(&row.9);
                params.push(&row.10);
                params.push(&row.11);
            }

            let sql = format!(
                "INSERT INTO transfers (chain_id, tx_hash, log_index, token, from_addr, to_addr, value, block_number, block_timestamp, swap_type, status, created_at)
                 VALUES {}
                 ON CONFLICT (chain_id, tx_hash, log_index)
                 DO UPDATE SET
                     status = EXCLUDED.status,
                     block_number = CASE WHEN EXCLUDED.block_number > 0 THEN EXCLUDED.block_number ELSE transfers.block_number END,
                     block_timestamp = CASE WHEN EXCLUDED.block_number > 0 THEN EXCLUDED.block_timestamp ELSE transfers.block_timestamp END
                 WHERE transfers.status != 'confirmed'",
                values_parts.join(", ")
            );

            let result = tx.execute(&sql, &params).await?;
            total_upserted += result as usize;
        }

        Ok(total_upserted)
    }

    /// Mark Bitcoin transfers as confirmed (pending→confirmed) for a given txid.
    pub async fn mark_transfers_confirmed(
        &self,
        chain_id: u32,
        tx_hash: &str,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<usize, DbError> {
        let client = self.pool.get().await?;
        let result = client.execute(
            "UPDATE transfers SET status = 'confirmed', block_number = $3, block_timestamp = $4
             WHERE chain_id = $1 AND tx_hash = $2 AND status = 'pending'",
            &[&(chain_id as i32), &tx_hash, &(block_number as i64), &(block_timestamp as i64)],
        ).await?;
        Ok(result as usize)
    }

    /// Batch-mark Bitcoin transfers as dropped for a list of txids.
    pub async fn mark_transfers_dropped(
        &self,
        chain_id: u32,
        tx_hashes: &[String],
    ) -> Result<usize, DbError> {
        if tx_hashes.is_empty() {
            return Ok(0);
        }
        let client = self.pool.get().await?;
        let result = client.execute(
            "UPDATE transfers SET status = 'dropped'
             WHERE chain_id = $1 AND tx_hash = ANY($2) AND status = 'pending'",
            &[&(chain_id as i32), &tx_hashes],
        ).await?;
        Ok(result as usize)
    }
}

#[derive(Default, Debug)]
pub struct CleanupStats {
    pub transfers_deleted: usize,
    pub fusion_plus_deleted: usize,
    pub fusion_deleted: usize,
    pub crypto2fiat_deleted: usize,
    pub metrics_deleted: usize,
}

/// Config override row from database (all fields optional — NULL = use default)
pub struct ConfigOverrideRow {
    pub chain_id: i32,
    pub blocks_per_request: Option<i32>,
    pub concurrent_fetches: Option<i32>,
    pub poll_interval_ms: Option<i64>,
    pub confirmation_blocks: Option<i32>,
    pub copy_threshold: Option<i32>,
    pub concurrent_inserts: Option<i32>,
}
