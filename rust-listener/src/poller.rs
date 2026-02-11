use crate::db::Database;
use crate::fusion::{
    compute_hashlock_from_secret, decode_crypto2fiat_event, decode_dst_escrow_created,
    decode_escrow_withdrawal, decode_order_filled, decode_src_escrow_created,
};
use crate::rpc::RpcClient;
use crate::types::{
    FusionPlusSwap, FusionSwap, Log, NetworkConfig, Transfer,
    ESCROW_FACTORY, SRC_ESCROW_CREATED_TOPIC, DST_ESCROW_CREATED_TOPIC,
    ESCROW_WITHDRAWAL_TOPIC, ESCROW_CANCELLED_TOPIC,
    AGGREGATION_ROUTER_V6, AGGREGATION_ROUTER_ZKSYNC,
    ORDER_FILLED_TOPIC, ORDER_CANCELLED_TOPIC,
    CRYPTO2FIAT_TOPIC,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
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
            poll_interval_ms: 500,   // Reduced from 2000 for real-time sync
            max_blocks_per_query: 500, // Increased from 50 for faster catch-up
            max_backfill_blocks: 500,
        }
    }
}

/// Data fetched from RPC for a block range (all owned, Send-safe for tokio::spawn)
struct FetchedData {
    from_block: u64,
    to_block: u64,
    fusion_plus_factory_logs: Vec<Log>,
    fusion_plus_escrow_logs: Vec<Log>,
    fusion_logs: Vec<Log>,
    crypto2fiat_logs: Vec<Log>,
    transfer_logs: Vec<Log>,
}

/// Fetch all logs for a block range with all 5 RPC calls in parallel.
/// Standalone function compatible with tokio::spawn (no &self, only owned/'static params).
async fn fetch_all_logs(
    rpc: Arc<RpcClient>,
    chain_id: u32,
    chain_name: &'static str,
    from_block: u64,
    to_block: u64,
) -> Result<FetchedData, String> {
    debug!(
        "[{}] Fetching all logs for blocks {} to {}",
        chain_name, from_block, to_block
    );

    // Determine router address based on chain
    let router_address = if chain_id == 324 {
        AGGREGATION_ROUTER_ZKSYNC
    } else {
        AGGREGATION_ROUTER_V6
    };

    // Fire ALL 5 RPC calls in parallel, wait for ALL to complete
    let (fp_factory_result, fp_escrow_result, fusion_result, c2f_result, transfer_result) = tokio::join!(
        // Fusion+ factory events (SrcEscrowCreated + DstEscrowCreated)
        rpc.get_logs_multi_topics(
            from_block, to_block, ESCROW_FACTORY,
            vec![SRC_ESCROW_CREATED_TOPIC.to_string(), DST_ESCROW_CREATED_TOPIC.to_string()],
        ),
        // Fusion+ escrow events (Withdrawal + Cancelled) from any address
        rpc.get_logs_multi_topics_any_address(
            from_block, to_block,
            vec![ESCROW_WITHDRAWAL_TOPIC.to_string(), ESCROW_CANCELLED_TOPIC.to_string()],
        ),
        // Fusion single-chain events (OrderFilled + OrderCancelled)
        rpc.get_logs_multi_topics(
            from_block, to_block, router_address,
            vec![ORDER_FILLED_TOPIC.to_string(), ORDER_CANCELLED_TOPIC.to_string()],
        ),
        // Crypto2Fiat events from any address
        rpc.get_logs_by_topic_any_address(from_block, to_block, CRYPTO2FIAT_TOPIC),
        // ERC20 Transfer events (the main data)
        rpc.get_transfer_logs(from_block, to_block),
    );

    // Fusion/c2f logs default to empty on error (non-critical)
    // Transfer logs propagate errors (critical - triggers adaptive step)
    Ok(FetchedData {
        from_block,
        to_block,
        fusion_plus_factory_logs: fp_factory_result.unwrap_or_default(),
        fusion_plus_escrow_logs: fp_escrow_result.unwrap_or_default(),
        fusion_logs: fusion_result.unwrap_or_default(),
        crypto2fiat_logs: c2f_result.unwrap_or_default(),
        transfer_logs: transfer_result.map_err(|e| format!("Failed to get transfer logs: {}", e))?,
    })
}

/// Per-chain poller that fetches Transfer events and stores them in PostgreSQL
pub struct ChainPoller {
    network: NetworkConfig,
    rpc: Arc<RpcClient>,
    db: Arc<Database>,  // Shared PostgreSQL database
    config: PollerConfig,
    block_timestamp_cache: HashMap<u64, u64>,
    /// Adaptive block step: starts at max_blocks_per_query, halves on "too big" errors
    current_block_step: u64,
}

impl ChainPoller {
    pub fn new(network: NetworkConfig, db: Arc<Database>) -> Self {
        Self::with_config(network, db, PollerConfig::default())
    }

    pub fn with_config(
        network: NetworkConfig,
        db: Arc<Database>,
        config: PollerConfig,
    ) -> Self {
        let rpc = Arc::new(RpcClient::new(&network.rpc_url, network.name));

        let initial_block_step = config.max_blocks_per_query;
        Self {
            network,
            rpc,
            db,
            config,
            block_timestamp_cache: HashMap::new(),
            current_block_step: initial_block_step,
        }
    }

    /// Check if an error string indicates a response-too-large or timeout error
    fn is_adaptive_step_error(err: &str) -> bool {
        let err_lower = err.to_lowercase();
        err_lower.contains("too big") || err_lower.contains("too large") || err.contains("-32008")
            || err_lower.contains("timed out") || err_lower.contains("timeout")
    }

    /// Run the poller loop with pipeline architecture:
    /// While processing cycle N (DB insert + fusion + checkpoint),
    /// concurrently prefetch cycle N+1's RPC data.
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

        // Prefetch handle for pipeline: holds the next cycle's RPC data
        let mut prefetch_handle: Option<JoinHandle<Result<FetchedData, String>>> = None;

        // Main polling loop
        loop {
            // ===== STEP 1: Get chain tip and compute block range =====
            let current_block = match self.rpc.get_block_number().await {
                Ok(b) => b,
                Err(e) => {
                    error!("[{}] Failed to get block number: {}", self.network.name, e);
                    // Discard stale prefetch
                    if let Some(handle) = prefetch_handle.take() {
                        handle.abort();
                    }
                    sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                    continue;
                }
            };

            let safe_to_block = current_block.saturating_sub(self.config.confirmation_blocks);
            let from_block = (last_processed_block + 1).max(
                last_processed_block
                    .saturating_sub(self.config.reorg_safety_blocks)
                    + 1,
            );

            // Skip if no new blocks
            if from_block > safe_to_block {
                // Discard stale prefetch
                if let Some(handle) = prefetch_handle.take() {
                    handle.abort();
                }
                sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                continue;
            }

            let actual_to_block = (from_block + self.current_block_step - 1).min(safe_to_block);

            debug!(
                "[{}] Polling blocks {} to {} (current: {})",
                self.network.name, from_block, actual_to_block, current_block
            );

            // ===== STEP 2: Get fetch result (from prefetch or fresh fetch) =====
            let fetch_result = if let Some(handle) = prefetch_handle.take() {
                match handle.await {
                    Ok(Ok(data)) if data.from_block == from_block => {
                        // Prefetch matches expected range, use it
                        debug!(
                            "[{}] Using prefetched data for blocks {}-{}",
                            self.network.name, data.from_block, data.to_block
                        );
                        Ok(data)
                    }
                    Ok(Ok(_)) => {
                        // Range mismatch (step changed or error recovery), fetch fresh
                        debug!(
                            "[{}] Prefetch range mismatch, fetching fresh",
                            self.network.name
                        );
                        fetch_all_logs(
                            Arc::clone(&self.rpc), self.network.chain_id,
                            self.network.name, from_block, actual_to_block,
                        ).await
                    }
                    Ok(Err(e)) => {
                        // Prefetch failed, try fresh fetch
                        debug!(
                            "[{}] Prefetch failed ({}), fetching fresh",
                            self.network.name, e
                        );
                        fetch_all_logs(
                            Arc::clone(&self.rpc), self.network.chain_id,
                            self.network.name, from_block, actual_to_block,
                        ).await
                    }
                    Err(_join_err) => {
                        // Task panicked or was cancelled, fetch fresh
                        fetch_all_logs(
                            Arc::clone(&self.rpc), self.network.chain_id,
                            self.network.name, from_block, actual_to_block,
                        ).await
                    }
                }
            } else {
                // No prefetch available, fetch synchronously
                fetch_all_logs(
                    Arc::clone(&self.rpc), self.network.chain_id,
                    self.network.name, from_block, actual_to_block,
                ).await
            };

            // ===== STEP 3: Process fetch result =====
            match fetch_result {
                Ok(data) => {
                    let data_to_block = data.to_block;

                    // Spawn prefetch for NEXT range (overlaps with process_fetched_data)
                    let next_from = data_to_block + 1;
                    let next_to = (next_from + self.current_block_step - 1).min(safe_to_block);
                    if next_from <= safe_to_block {
                        let rpc_clone = Arc::clone(&self.rpc);
                        let chain_id = self.network.chain_id;
                        let chain_name = self.network.name;
                        prefetch_handle = Some(tokio::spawn(async move {
                            fetch_all_logs(rpc_clone, chain_id, chain_name, next_from, next_to).await
                        }));
                    }

                    // Process current data (sequential: insert → fusion → checkpoint)
                    match self.process_fetched_data(data, &mut last_processed_block).await {
                        Ok(events_processed) => {
                            if events_processed > 0 {
                                debug!(
                                    "[{}] Processed {} events, checkpoint: {}",
                                    self.network.name, events_processed, last_processed_block
                                );
                            }
                            // Gradually restore block step toward max after success
                            if self.current_block_step < self.config.max_blocks_per_query {
                                self.current_block_step = (self.current_block_step * 2).min(self.config.max_blocks_per_query);
                                info!(
                                    "[{}] Increased block step to {}",
                                    self.network.name, self.current_block_step
                                );
                            }
                        }
                        Err(e) => {
                            // ABORT prefetch on process error (step may change, range invalid)
                            if let Some(handle) = prefetch_handle.take() {
                                handle.abort();
                            }
                            // Adaptive step reduction
                            if Self::is_adaptive_step_error(&e) {
                                let old_step = self.current_block_step;
                                self.current_block_step = (self.current_block_step / 2).max(1);
                                warn!(
                                    "[{}] Response too large, reducing block step from {} to {}",
                                    self.network.name, old_step, self.current_block_step
                                );
                            }
                            error!("[{}] Process error: {}", self.network.name, e);
                        }
                    }
                }
                Err(e) => {
                    // Fetch failed - adaptive step reduction
                    if Self::is_adaptive_step_error(&e) {
                        let old_step = self.current_block_step;
                        self.current_block_step = (self.current_block_step / 2).max(1);
                        warn!(
                            "[{}] Response too large, reducing block step from {} to {}",
                            self.network.name, old_step, self.current_block_step
                        );
                    }
                    error!("[{}] Fetch error: {}", self.network.name, e);
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
            .await
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
                    .await
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
                .await
                .map_err(|e| format!("DB error: {}", e))?;
            start_block
        };

        Ok(start_block)
    }

    // =========================================================================
    // Data Processing (DB insert + fusion processing + checkpoint)
    // =========================================================================

    /// Process previously fetched data: build transfers, insert to DB,
    /// process fusion/c2f events, set checkpoint.
    /// This runs sequentially on &mut self (never spawned to a separate task).
    async fn process_fetched_data(
        &mut self,
        data: FetchedData,
        last_processed_block: &mut u64,
    ) -> Result<usize, String> {
        // =====================================================================
        // Build swap_type map from fusion/c2f logs
        // =====================================================================
        let mut swap_type_map: HashMap<String, &'static str> = HashMap::new();

        for log in &data.fusion_plus_factory_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion_plus");
        }
        for log in &data.fusion_plus_escrow_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion_plus");
        }
        for log in &data.fusion_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion");
        }
        for log in &data.crypto2fiat_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "crypto_to_fiat");
        }

        // =====================================================================
        // Process transfers and insert with swap_type from map
        // =====================================================================
        if !data.transfer_logs.is_empty() {
            info!(
                "[{}] Found {} Transfer events in blocks {}-{}",
                self.network.name,
                data.transfer_logs.len(),
                data.from_block,
                data.to_block
            );
        }

        let mut transfers = Vec::with_capacity(data.transfer_logs.len());

        for log in &data.transfer_logs {
            // Validate Transfer event structure
            if log.topics.len() < 3 {
                continue; // Invalid Transfer event
            }

            let block_number = log.block_number_u64();
            let timestamp = self.get_block_timestamp(block_number).await?;

            // Look up swap_type from the map
            let swap_type = swap_type_map.get(&log.transaction_hash.to_lowercase()).map(|s| s.to_string());

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
                swap_type,
            };

            transfers.push(transfer);
        }

        // Batch insert to PostgreSQL database (with swap_type already set)
        let inserted = if !transfers.is_empty() {
            self.db
                .insert_transfers_batch(self.network.chain_id, &transfers)
                .await
                .map_err(|e| format!("DB error: {}", e))?
        } else {
            0
        };

        // =====================================================================
        // Process fusion events (insert swap records, no UPDATE needed)
        // Must run AFTER insert_transfers_batch (fusion needs transfers in DB)
        // =====================================================================
        let fusion_plus_events = self.process_fusion_plus_logs(
            &data.fusion_plus_factory_logs, &data.fusion_plus_escrow_logs
        ).await?;
        let fusion_events = self.process_fusion_logs(&data.fusion_logs).await?;
        let crypto2fiat_events = self.process_crypto2fiat_logs(&data.crypto2fiat_logs).await?;

        // =====================================================================
        // Update checkpoint (ONLY after all DB work completes)
        // =====================================================================
        *last_processed_block = data.to_block;
        self.db
            .set_checkpoint(self.network.chain_id, data.to_block)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

        Ok(inserted + fusion_plus_events + fusion_events + crypto2fiat_events)
    }

    // =========================================================================
    // Log Processing Methods (process pre-fetched logs)
    // =========================================================================

    /// Process Fusion+ logs (factory and escrow events)
    async fn process_fusion_plus_logs(
        &mut self,
        factory_logs: &[Log],
        escrow_logs: &[Log],
    ) -> Result<usize, String> {
        let mut events_processed = 0;

        for log in factory_logs {
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

        for log in escrow_logs {
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
                "[{}] Processed {} Fusion+ events",
                self.network.name, events_processed
            );
        }

        Ok(events_processed)
    }

    /// Process Fusion (single-chain) logs
    async fn process_fusion_logs(&mut self, logs: &[Log]) -> Result<usize, String> {
        let mut events_processed = 0;

        for log in logs {
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
                "[{}] Processed {} Fusion events",
                self.network.name, events_processed
            );
        }

        Ok(events_processed)
    }

    /// Process Crypto2Fiat logs
    async fn process_crypto2fiat_logs(&mut self, logs: &[Log]) -> Result<usize, String> {
        let mut events_processed = 0;

        for log in logs {
            if log.topics.is_empty() {
                continue;
            }

            let timestamp = self.get_block_timestamp(log.block_number_u64()).await?;

            if let Err(e) = self.process_crypto2fiat_event(log, timestamp).await {
                debug!("[{}] Failed to process Crypto2Fiat event: {}", self.network.name, e);
            } else {
                events_processed += 1;
            }
        }

        if events_processed > 0 {
            info!(
                "[{}] Processed {} Crypto2Fiat events",
                self.network.name, events_processed
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

        // Insert the swap into database
        self.db
            .insert_fusion_plus_swap(&swap)
            .await
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
            .await
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
        if let Ok(Some(swap)) = self.db.get_fusion_plus_swap_by_hashlock(&hashlock).await {
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
                .await
                .map_err(|e| format!("DB error: {}", e))?;

            if updated {
                let side = if is_src { "source" } else { "destination" };
                info!(
                    "[{}] Fusion+ {} withdrawal: order_hash={} secret={} tx={}",
                    self.network.name, side, swap.order_hash, secret, log.transaction_hash
                );
            }
        }

        debug!(
            "[{}] Fusion+ withdrawal from escrow {} with hashlock {}",
            self.network.name, log.address, hashlock
        );

        Ok(())
    }

    /// Process EscrowCancelled event
    async fn process_escrow_cancelled(&self, log: &Log, _timestamp: u64) -> Result<(), String> {
        debug!(
            "[{}] Fusion+ escrow cancelled: {}",
            self.network.name, log.address
        );

        Ok(())
    }

    // =========================================================================
    // Fusion (Single-Chain) Methods
    // =========================================================================

    /// Process OrderFilled or OrderCancelled event
    async fn process_order_filled(&self, log: &Log, timestamp: u64, status: &str) -> Result<(), String> {
        let data = decode_order_filled(&log.topics, &log.data)
            .ok_or_else(|| "Failed to decode OrderFilled data".to_string())?;

        // Check if remaining > 0 (partial fill)
        let remaining_hex = data.remaining.trim_start_matches("0x");
        let is_partial = !remaining_hex.chars().all(|c| c == '0');

        // Get first and last transfers to populate maker/taker info
        // First transfer = maker sends maker_token (maker = from_addr of first transfer)
        // Last transfer = taker receives taker_token (taker = to_addr of last transfer)
        let (maker, taker, maker_token, taker_token, maker_amount, taker_amount) =
            match self.db.get_first_last_transfers(self.network.chain_id, &log.transaction_hash).await {
                Ok(Some((first, last))) => {
                    (
                        first.from_addr.clone(),         // maker = sender of first transfer
                        Some(last.to_addr.clone()),      // taker = recipient of last transfer
                        Some(first.token.clone()),       // maker_token = token of first transfer
                        Some(last.token.clone()),        // taker_token = token of last transfer
                        Some(first.value.clone()),       // maker_amount = value of first transfer
                        Some(last.value.clone()),        // taker_amount = value of last transfer
                    )
                }
                Ok(None) => {
                    // No transfers found for this tx (shouldn't happen normally)
                    (String::new(), None, None, None, None, None)
                }
                Err(e) => {
                    warn!("[{}] Failed to get transfers for fusion swap: {}", self.network.name, e);
                    (String::new(), None, None, None, None, None)
                }
            };

        let swap = FusionSwap {
            order_hash: data.order_hash.clone(),
            chain_id: self.network.chain_id,
            tx_hash: log.transaction_hash.clone(),
            block_number: log.block_number_u64(),
            block_timestamp: timestamp,
            log_index: log.log_index_u32(),
            maker,
            taker,
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
            .await
            .map_err(|e| format!("DB error: {}", e))?;

        info!(
            "[{}] Fusion {} order: order_hash={} maker={} taker={:?} tx={}",
            self.network.name, status, data.order_hash, swap.maker, swap.taker, log.transaction_hash
        );

        Ok(())
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
        let cutoff = current_block.saturating_sub(200);
        let before = self.block_timestamp_cache.len();
        self.block_timestamp_cache
            .retain(|&block, _| block >= cutoff);
        // Reclaim memory if we removed entries
        if self.block_timestamp_cache.len() < before {
            self.block_timestamp_cache.shrink_to_fit();
        }
    }

    // =========================================================================
    // Crypto2Fiat Methods (KentuckyDelegate)
    // =========================================================================

    /// Process a Crypto2Fiat event
    async fn process_crypto2fiat_event(&self, log: &Log, timestamp: u64) -> Result<(), String> {
        let mut event = decode_crypto2fiat_event(log)
            .ok_or_else(|| "Failed to decode Crypto2Fiat event".to_string())?;

        // Fill in chain/tx details
        event.chain_id = self.network.chain_id;
        event.tx_hash = log.transaction_hash.clone();
        event.block_number = log.block_number_u64();
        event.block_timestamp = timestamp;
        event.log_index = log.log_index_u32();

        // Insert the event
        self.db
            .insert_crypto2fiat_event(&event)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

        info!(
            "[{}] Crypto2Fiat: order_id={} token={} amount={} recipient={} tx={}",
            self.network.name, event.order_id, event.token, event.amount, event.recipient, event.tx_hash
        );

        Ok(())
    }
}
