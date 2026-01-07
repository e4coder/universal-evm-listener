use crate::db::Database;
use crate::rpc::RpcClient;
use crate::types::{NetworkConfig, Transfer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Configuration for the chain poller
pub struct PollerConfig {
    /// Number of blocks to look back for reorg safety
    pub reorg_safety_blocks: u64,
    /// Number of confirmations before processing a block
    pub confirmation_blocks: u64,
    /// Polling interval in milliseconds
    pub poll_interval_ms: u64,
    /// Maximum blocks to query in a single getLogs call
    pub max_blocks_per_query: u64,
    /// Maximum blocks to backfill on startup
    pub max_backfill_blocks: u64,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            reorg_safety_blocks: 10,
            confirmation_blocks: 3,
            poll_interval_ms: 2000,
            max_blocks_per_query: 100,
            max_backfill_blocks: 500,
        }
    }
}

/// Per-chain poller that fetches Transfer events and stores them in SQLite
pub struct ChainPoller {
    network: NetworkConfig,
    rpc: RpcClient,
    db: Arc<Database>,
    config: PollerConfig,
    block_timestamp_cache: HashMap<u64, u64>,
}

impl ChainPoller {
    pub fn new(network: NetworkConfig, db: Arc<Database>) -> Self {
        Self::with_config(network, db, PollerConfig::default())
    }

    pub fn with_config(network: NetworkConfig, db: Arc<Database>, config: PollerConfig) -> Self {
        let rpc = RpcClient::new(&network.rpc_url, network.name);

        Self {
            network,
            rpc,
            db,
            config,
            block_timestamp_cache: HashMap::new(),
        }
    }

    /// Run the poller loop
    pub async fn run(&mut self) {
        info!(
            "[{}] Starting poller (chain_id: {})",
            self.network.name, self.network.chain_id
        );

        // Get starting block
        let mut last_processed_block = match self.initialize_checkpoint().await {
            Ok(block) => block,
            Err(e) => {
                error!("[{}] Failed to initialize: {}", self.network.name, e);
                return;
            }
        };

        info!(
            "[{}] Starting from block {}",
            self.network.name, last_processed_block
        );

        // Main polling loop
        loop {
            match self.poll_once(&mut last_processed_block).await {
                Ok(events_processed) => {
                    if events_processed > 0 {
                        debug!(
                            "[{}] Processed {} events, checkpoint: {}",
                            self.network.name, events_processed, last_processed_block
                        );
                    }
                }
                Err(e) => {
                    error!("[{}] Poll error: {}", self.network.name, e);
                    // Continue polling after error, don't crash
                }
            }

            // Clean up old cached timestamps
            self.cleanup_timestamp_cache(last_processed_block);

            sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
        }
    }

    /// Initialize checkpoint - get starting block
    async fn initialize_checkpoint(&self) -> Result<u64, String> {
        // Get current block from chain
        let current_block = self
            .rpc
            .get_block_number()
            .await
            .map_err(|e| format!("Failed to get block number: {}", e))?;

        // Check for saved checkpoint
        let saved_checkpoint = self
            .db
            .get_checkpoint(self.network.chain_id)
            .map_err(|e| format!("DB error: {}", e))?;

        let start_block = if let Some(checkpoint) = saved_checkpoint {
            let blocks_behind = current_block.saturating_sub(checkpoint);

            if blocks_behind > self.config.max_backfill_blocks {
                // Checkpoint too old - skip to recent blocks
                let new_start = current_block.saturating_sub(self.config.reorg_safety_blocks);
                warn!(
                    "[{}] Checkpoint {} is {} blocks behind (max: {}). Skipping to block {}",
                    self.network.name,
                    checkpoint,
                    blocks_behind,
                    self.config.max_backfill_blocks,
                    new_start
                );
                self.db
                    .set_checkpoint(self.network.chain_id, new_start)
                    .map_err(|e| format!("DB error: {}", e))?;
                new_start
            } else {
                info!(
                    "[{}] Found checkpoint at block {} ({} blocks behind)",
                    self.network.name, checkpoint, blocks_behind
                );
                checkpoint
            }
        } else {
            // First start - begin from current block minus safety margin
            let start_block = current_block.saturating_sub(self.config.reorg_safety_blocks);
            info!(
                "[{}] First start, beginning from block {}",
                self.network.name, start_block
            );
            self.db
                .set_checkpoint(self.network.chain_id, start_block)
                .map_err(|e| format!("DB error: {}", e))?;
            start_block
        };

        Ok(start_block)
    }

    /// Poll for new events once
    async fn poll_once(&mut self, last_processed_block: &mut u64) -> Result<usize, String> {
        // Get current block
        let current_block = self
            .rpc
            .get_block_number()
            .await
            .map_err(|e| format!("Failed to get block number: {}", e))?;

        // Calculate safe block range
        let to_block = current_block.saturating_sub(self.config.confirmation_blocks);
        let from_block = (*last_processed_block + 1).max(
            last_processed_block
                .saturating_sub(self.config.reorg_safety_blocks)
                + 1,
        );

        // Skip if no new blocks
        if from_block > to_block {
            return Ok(0);
        }

        // Limit query size
        let actual_to_block = (from_block + self.config.max_blocks_per_query - 1).min(to_block);

        debug!(
            "[{}] Polling blocks {} to {} (current: {})",
            self.network.name, from_block, actual_to_block, current_block
        );

        // Fetch Transfer events
        let logs = self
            .rpc
            .get_transfer_logs(from_block, actual_to_block)
            .await
            .map_err(|e| format!("Failed to get logs: {}", e))?;

        if !logs.is_empty() {
            info!(
                "[{}] Found {} Transfer events in blocks {}-{}",
                self.network.name,
                logs.len(),
                from_block,
                actual_to_block
            );
        }

        // Process logs into transfers
        let mut transfers = Vec::with_capacity(logs.len());

        for log in &logs {
            // Validate Transfer event structure
            if log.topics.len() < 3 {
                continue; // Invalid Transfer event
            }

            let block_number = log.block_number_u64();
            let timestamp = self.get_block_timestamp(block_number).await?;

            let transfer = Transfer {
                chain_id: self.network.chain_id,
                tx_hash: log.transaction_hash.clone(),
                log_index: log.log_index_u32(),
                token: log.address.to_lowercase(),
                from_addr: format!("0x{}", &log.topics[1][26..]), // Remove padding
                to_addr: format!("0x{}", &log.topics[2][26..]),   // Remove padding
                value: log.data.clone(),
                block_number,
                block_timestamp: timestamp,
            };

            transfers.push(transfer);
        }

        // Batch insert to SQLite
        let inserted = if !transfers.is_empty() {
            self.db
                .insert_transfers_batch(&transfers)
                .map_err(|e| format!("DB error: {}", e))?
        } else {
            0
        };

        // Update checkpoint
        *last_processed_block = actual_to_block;
        self.db
            .set_checkpoint(self.network.chain_id, actual_to_block)
            .map_err(|e| format!("DB error: {}", e))?;

        Ok(inserted)
    }

    /// Get block timestamp with caching
    async fn get_block_timestamp(&mut self, block_number: u64) -> Result<u64, String> {
        // Check cache first
        if let Some(&timestamp) = self.block_timestamp_cache.get(&block_number) {
            return Ok(timestamp);
        }

        // Fetch from RPC
        let block = self
            .rpc
            .get_block(block_number)
            .await
            .map_err(|e| format!("Failed to get block {}: {}", block_number, e))?;

        let timestamp = block.timestamp_u64();

        // Cache it
        self.block_timestamp_cache.insert(block_number, timestamp);

        Ok(timestamp)
    }

    /// Clean up old entries from timestamp cache
    fn cleanup_timestamp_cache(&mut self, current_block: u64) {
        // Keep only blocks within the last 200 blocks
        let cutoff = current_block.saturating_sub(200);
        self.block_timestamp_cache
            .retain(|&block, _| block >= cutoff);
    }
}
