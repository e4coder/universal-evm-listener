use crate::db::Database;
use crate::fusion::{
    compute_hashlock_from_secret, decode_dst_escrow_created, decode_escrow_withdrawal,
    decode_order_filled, decode_src_escrow_created,
};
use crate::rpc::RpcClient;
use crate::types::{
    FusionPlusSwap, FusionSwap, Log, NetworkConfig, Transfer,
    ESCROW_FACTORY, SRC_ESCROW_CREATED_TOPIC, DST_ESCROW_CREATED_TOPIC,
    ESCROW_WITHDRAWAL_TOPIC, ESCROW_CANCELLED_TOPIC,
    AGGREGATION_ROUTER_V6, AGGREGATION_ROUTER_ZKSYNC,
    ORDER_FILLED_TOPIC, ORDER_CANCELLED_TOPIC,
};
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

        // Fetch and process Fusion+ events from EscrowFactory
        let fusion_plus_events = self.poll_fusion_plus_events(from_block, actual_to_block).await?;

        // Fetch and process Fusion (single-chain) events from LimitOrderProtocol
        let fusion_events = self.poll_fusion_events(from_block, actual_to_block).await?;

        // Update checkpoint
        *last_processed_block = actual_to_block;
        self.db
            .set_checkpoint(self.network.chain_id, actual_to_block)
            .map_err(|e| format!("DB error: {}", e))?;

        Ok(inserted + fusion_plus_events + fusion_events)
    }

    /// Poll for Fusion+ events from EscrowFactory contract
    async fn poll_fusion_plus_events(
        &mut self,
        from_block: u64,
        to_block: u64,
    ) -> Result<usize, String> {
        // Fetch SrcEscrowCreated and DstEscrowCreated events from EscrowFactory
        let factory_topics = vec![
            SRC_ESCROW_CREATED_TOPIC.to_string(),
            DST_ESCROW_CREATED_TOPIC.to_string(),
        ];

        let factory_logs = self
            .rpc
            .get_logs_multi_topics(from_block, to_block, ESCROW_FACTORY, factory_topics)
            .await
            .unwrap_or_default();

        let mut events_processed = 0;

        for log in &factory_logs {
            if log.topics.is_empty() {
                continue;
            }

            let timestamp = self.get_block_timestamp(log.block_number_u64()).await?;

            if log.topics[0].to_lowercase() == SRC_ESCROW_CREATED_TOPIC {
                if let Err(e) = self.process_src_escrow_created(log, timestamp).await {
                    warn!("[{}] Failed to process SrcEscrowCreated: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            } else if log.topics[0].to_lowercase() == DST_ESCROW_CREATED_TOPIC {
                if let Err(e) = self.process_dst_escrow_created(log, timestamp).await {
                    warn!("[{}] Failed to process DstEscrowCreated: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            }
        }

        // Fetch EscrowWithdrawal and EscrowCancelled events (from any escrow contract)
        let escrow_topics = vec![
            ESCROW_WITHDRAWAL_TOPIC.to_string(),
            ESCROW_CANCELLED_TOPIC.to_string(),
        ];

        // Note: We can't filter by address for escrow events since escrow addresses vary
        // So we fetch by topic only using OR filter
        let escrow_logs = self
            .rpc
            .get_logs_multi_topics_any_address(from_block, to_block, escrow_topics)
            .await
            .unwrap_or_default();

        for log in &escrow_logs {
            if log.topics.is_empty() {
                continue;
            }

            let timestamp = self.get_block_timestamp(log.block_number_u64()).await?;

            if log.topics[0].to_lowercase() == ESCROW_WITHDRAWAL_TOPIC {
                if let Err(e) = self.process_escrow_withdrawal(log, timestamp).await {
                    debug!("[{}] Failed to process EscrowWithdrawal: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            } else if log.topics[0].to_lowercase() == ESCROW_CANCELLED_TOPIC {
                if let Err(e) = self.process_escrow_cancelled(log, timestamp).await {
                    debug!("[{}] Failed to process EscrowCancelled: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            }
        }

        if events_processed > 0 {
            info!(
                "[{}] Processed {} Fusion+ events in blocks {}-{}",
                self.network.name, events_processed, from_block, to_block
            );
        }

        Ok(events_processed)
    }

    /// Process SrcEscrowCreated event
    async fn process_src_escrow_created(&self, log: &Log, timestamp: u64) -> Result<(), String> {
        let data = decode_src_escrow_created(&log.data)
            .ok_or_else(|| "Failed to decode SrcEscrowCreated data".to_string())?;

        // Create new swap record
        let swap = FusionPlusSwap::from_src_created(
            &data,
            self.network.chain_id,
            &log.transaction_hash,
            log.block_number_u64(),
            timestamp,
            log.log_index_u32(),
        );

        // Insert the swap
        self.db
            .insert_fusion_plus_swap(&swap)
            .map_err(|e| format!("DB error: {}", e))?;

        // Label all transfers in this tx as fusion_plus
        self.db
            .label_transfers_as_fusion(self.network.chain_id, &log.transaction_hash, "fusion_plus")
            .map_err(|e| format!("DB error: {}", e))?;

        info!(
            "[{}] Fusion+ SrcEscrow created: order_hash={} dst_chain={}",
            self.network.name, data.order_hash, data.dst_chain_id
        );

        Ok(())
    }

    /// Process DstEscrowCreated event
    async fn process_dst_escrow_created(&self, log: &Log, timestamp: u64) -> Result<(), String> {
        let data = decode_dst_escrow_created(&log.data)
            .ok_or_else(|| "Failed to decode DstEscrowCreated data".to_string())?;

        // Update existing swap with destination data
        let updated = self.db
            .update_fusion_plus_dst(
                &data.order_hash,
                &data,
                self.network.chain_id,
                &log.transaction_hash,
                log.block_number_u64(),
                timestamp,
                log.log_index_u32(),
                Some(&log.address),
            )
            .map_err(|e| format!("DB error: {}", e))?;

        // Label all transfers in this tx as fusion_plus
        self.db
            .label_transfers_as_fusion(self.network.chain_id, &log.transaction_hash, "fusion_plus")
            .map_err(|e| format!("DB error: {}", e))?;

        if updated {
            info!(
                "[{}] Fusion+ DstEscrow created: order_hash={}",
                self.network.name, data.order_hash
            );
        } else {
            debug!(
                "[{}] Fusion+ DstEscrow created for unknown order: {}",
                self.network.name, data.order_hash
            );
        }

        Ok(())
    }

    /// Process EscrowWithdrawal event
    async fn process_escrow_withdrawal(&self, log: &Log, timestamp: u64) -> Result<(), String> {
        let secret = decode_escrow_withdrawal(&log.data)
            .ok_or_else(|| "Failed to decode EscrowWithdrawal data".to_string())?;

        // Compute hashlock from secret: hashlock = keccak256(secret)
        let hashlock = compute_hashlock_from_secret(&secret)
            .ok_or_else(|| "Failed to compute hashlock from secret".to_string())?;

        // Look up the swap by hashlock and update its status
        if let Ok(Some(swap)) = self.db.get_fusion_plus_swap_by_hashlock(&hashlock) {
            // Determine if this is src or dst withdrawal based on chain_id
            let is_src = swap.src_chain_id == self.network.chain_id;

            // Update the swap status with secret and tx details
            let updated = self.db
                .update_fusion_plus_withdrawal_by_hashlock(
                    &hashlock,
                    self.network.chain_id,
                    is_src,
                    &secret,
                    &log.transaction_hash,
                    log.block_number_u64(),
                    timestamp,
                    log.log_index_u32(),
                )
                .map_err(|e| format!("DB error: {}", e))?;

            if updated {
                let side = if is_src { "source" } else { "destination" };
                info!(
                    "[{}] Fusion+ {} withdrawal: order_hash={} secret={} tx={}",
                    self.network.name, side, swap.order_hash, secret, log.transaction_hash
                );
            }
        }

        // Label transfers in this tx as fusion_plus
        self.db
            .label_transfers_as_fusion(self.network.chain_id, &log.transaction_hash, "fusion_plus")
            .map_err(|e| format!("DB error: {}", e))?;

        debug!(
            "[{}] Fusion+ withdrawal from escrow {} with hashlock {}",
            self.network.name, log.address, hashlock
        );

        Ok(())
    }

    /// Process EscrowCancelled event
    async fn process_escrow_cancelled(&self, log: &Log, _timestamp: u64) -> Result<(), String> {
        // Similar to withdrawal, we'd need to track escrow addresses to update the swap record
        // For now, just label the transfers

        self.db
            .label_transfers_as_fusion(self.network.chain_id, &log.transaction_hash, "fusion_plus")
            .map_err(|e| format!("DB error: {}", e))?;

        debug!(
            "[{}] Fusion+ escrow cancelled: {}",
            self.network.name, log.address
        );

        Ok(())
    }

    // =========================================================================
    // Fusion (Single-Chain) Methods
    // =========================================================================

    /// Poll for Fusion events from Aggregation Router V6
    async fn poll_fusion_events(
        &mut self,
        from_block: u64,
        to_block: u64,
    ) -> Result<usize, String> {
        // Determine contract address based on chain
        let router_address = if self.network.chain_id == 324 {
            // zkSync Era
            AGGREGATION_ROUTER_ZKSYNC
        } else {
            AGGREGATION_ROUTER_V6
        };

        // Fetch OrderFilled and OrderCancelled events
        let topics = vec![
            ORDER_FILLED_TOPIC.to_string(),
            ORDER_CANCELLED_TOPIC.to_string(),
        ];

        let logs = self
            .rpc
            .get_logs_multi_topics(from_block, to_block, router_address, topics)
            .await
            .unwrap_or_default();

        let mut events_processed = 0;

        for log in &logs {
            if log.topics.is_empty() {
                continue;
            }

            let timestamp = self.get_block_timestamp(log.block_number_u64()).await?;
            let topic0 = log.topics[0].to_lowercase();

            if topic0 == ORDER_FILLED_TOPIC {
                if let Err(e) = self.process_order_filled(log, timestamp, "filled").await {
                    debug!("[{}] Failed to process OrderFilled: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            } else if topic0 == ORDER_CANCELLED_TOPIC {
                if let Err(e) = self.process_order_filled(log, timestamp, "cancelled").await {
                    debug!("[{}] Failed to process OrderCancelled: {}", self.network.name, e);
                } else {
                    events_processed += 1;
                }
            }
        }

        if events_processed > 0 {
            info!(
                "[{}] Processed {} Fusion events in blocks {}-{}",
                self.network.name, events_processed, from_block, to_block
            );
        }

        Ok(events_processed)
    }

    /// Process OrderFilled or OrderCancelled event
    async fn process_order_filled(&self, log: &Log, timestamp: u64, status: &str) -> Result<(), String> {
        let data = decode_order_filled(&log.topics, &log.data)
            .ok_or_else(|| "Failed to decode OrderFilled data".to_string())?;

        // Check if remaining > 0 (partial fill)
        let remaining_hex = data.remaining.trim_start_matches("0x");
        let is_partial = !remaining_hex.chars().all(|c| c == '0');

        // Try to enrich swap data from transfers in the same transaction
        let (maker, maker_token, taker_token, maker_amount, taker_amount) =
            self.extract_swap_details_from_transfers(&log.transaction_hash);

        let swap = FusionSwap {
            order_hash: data.order_hash.clone(),
            chain_id: self.network.chain_id,
            tx_hash: log.transaction_hash.clone(),
            block_number: log.block_number_u64(),
            block_timestamp: timestamp,
            log_index: log.log_index_u32(),
            maker,
            maker_token,
            taker_token,
            maker_amount,
            taker_amount,
            remaining: data.remaining.clone(),
            is_partial_fill: is_partial,
            status: status.to_string(),
        };

        // Insert swap record
        self.db
            .insert_fusion_swap(&swap)
            .map_err(|e| format!("DB error: {}", e))?;

        // Label all transfers in this tx as 'fusion'
        self.db
            .label_transfers_as_fusion(self.network.chain_id, &log.transaction_hash, "fusion")
            .map_err(|e| format!("DB error: {}", e))?;

        info!(
            "[{}] Fusion {} order: order_hash={} maker={} tx={}",
            self.network.name, status, data.order_hash, swap.maker, log.transaction_hash
        );

        Ok(())
    }

    /// Extract swap details (maker, tokens, amounts) from transfers in the same transaction
    ///
    /// Logic: Find an address that both sent and received tokens in the same tx.
    /// That address is the maker (user). The token they sent is maker_token,
    /// the token they received is taker_token.
    fn extract_swap_details_from_transfers(
        &self,
        tx_hash: &str,
    ) -> (String, Option<String>, Option<String>, Option<String>, Option<String>) {
        // Get all transfers in this transaction
        let transfers = match self.db.get_transfers_by_tx_hash(self.network.chain_id, tx_hash) {
            Ok(t) => t,
            Err(_) => return (String::new(), None, None, None, None),
        };

        if transfers.is_empty() {
            return (String::new(), None, None, None, None);
        }

        // Build maps of who sent what and who received what
        use std::collections::HashMap;
        let mut sent_by: HashMap<String, Vec<(String, String)>> = HashMap::new(); // addr -> [(token, value)]
        let mut received_by: HashMap<String, Vec<(String, String)>> = HashMap::new(); // addr -> [(token, value)]

        for t in &transfers {
            sent_by
                .entry(t.from_addr.clone())
                .or_default()
                .push((t.token.clone(), t.value.clone()));
            received_by
                .entry(t.to_addr.clone())
                .or_default()
                .push((t.token.clone(), t.value.clone()));
        }

        // Find an address that both sent and received tokens (the maker/user)
        // Exclude common router/aggregator addresses
        let router_addresses: Vec<&str> = vec![
            "0x111111125421ca6dc452d289314280a0f8842a65", // Aggregation Router V6
            "0x1111111254eeb25477b68fb85ed929f73a960582", // Aggregation Router V5
            "0x6fd4383cb451173d5f9304f041c7bcbf27d561ff", // zkSync Router
        ];

        for (addr, sent_tokens) in &sent_by {
            // Skip router addresses
            if router_addresses.contains(&addr.as_str()) {
                continue;
            }

            if let Some(received_tokens) = received_by.get(addr) {
                // This address both sent and received - likely the maker
                // Get the first token sent and first token received
                if let (Some((maker_token, maker_amount)), Some((taker_token, taker_amount))) =
                    (sent_tokens.first(), received_tokens.first())
                {
                    // Make sure they're different tokens (actual swap, not just routing)
                    if maker_token != taker_token {
                        return (
                            addr.clone(),
                            Some(maker_token.clone()),
                            Some(taker_token.clone()),
                            Some(maker_amount.clone()),
                            Some(taker_amount.clone()),
                        );
                    }
                }
            }
        }

        // Fallback: if no clear maker found, return empty
        (String::new(), None, None, None, None)
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
