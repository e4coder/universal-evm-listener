use crate::db::Database;
use crate::fusion::{
    compute_hashlock_from_secret, decode_crypto2fiat_event, decode_dst_escrow_created,
    decode_escrow_withdrawal, decode_order_filled, decode_src_escrow_created,
};
use crate::rpc::RpcClient;
use crate::types::{
    ChainStats, FusionPlusSwap, FusionSwap, LiveConfig, Log, NetworkConfig, Transfer,
    ESCROW_FACTORY, SRC_ESCROW_CREATED_TOPIC, DST_ESCROW_CREATED_TOPIC,
    ESCROW_WITHDRAWAL_TOPIC, ESCROW_CANCELLED_TOPIC,
    AGGREGATION_ROUTER_V6, AGGREGATION_ROUTER_ZKSYNC,
    ORDER_FILLED_TOPIC, ORDER_CANCELLED_TOPIC,
    CRYPTO2FIAT_TOPIC,
};
use std::sync::atomic::Ordering::Relaxed;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{sleep, Instant};
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
    /// Maximum number of concurrent in-flight fetches
    pub max_concurrent_fetches: usize,
    /// Bounded channel capacity for fetcher->processor communication
    pub channel_capacity: usize,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            reorg_safety_blocks: 10,
            confirmation_blocks: 3,
            poll_interval_ms: 500,
            max_blocks_per_query: 50,
            max_backfill_blocks: 500,
            max_concurrent_fetches: 10,
            channel_capacity: 15,
        }
    }
}

// FetcherConfig removed — fetcher_loop now reads from Arc<LiveConfig> atomics

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

// =============================================================================
// Fetcher Loop (Producer) - standalone function, spawned as independent task
// =============================================================================

/// Continuously fetches block ranges and sends results to the processor via channel.
/// Manages its own adaptive block step and concurrent in-flight fetches.
async fn fetcher_loop(
    rpc: Arc<RpcClient>,
    slow_rpc: Arc<RpcClient>,
    chain_id: u32,
    chain_name: &'static str,
    start_from: u64,
    live_config: Arc<LiveConfig>,
    tx: mpsc::Sender<FetchedData>,
    stats: Arc<ChainStats>,
) {
    let mut next_from = start_from + 1;
    let mut join_set: JoinSet<(u64, u64, Result<FetchedData, String>)> = JoinSet::new();
    let mut pending_ranges: VecDeque<(u64, u64)> = VecDeque::new();
    let mut last_chance_ranges: HashSet<(u64, u64)> = HashSet::new();
    let mut cached_safe_tip: u64 = 0;
    let mut tip_fetched_at = Instant::now() - Duration::from_secs(10); // force initial refresh

    info!("[{}] Fetcher started (live config)", chain_name);

    loop {
        // Read live config values at the start of each cycle
        let step = live_config.max_blocks_per_query.load(Relaxed);
        let confirmation_blocks = live_config.confirmation_blocks.load(Relaxed);
        let max_concurrent = live_config.max_concurrent_fetches.load(Relaxed);
        let poll_interval = Duration::from_millis(live_config.poll_interval_ms.load(Relaxed));

        // 1. Refresh chain tip if stale or caught up with no pending work.
        let caught_up = next_from > cached_safe_tip
            && pending_ranges.is_empty()
            && last_chance_ranges.is_empty()
            && join_set.is_empty();
        let stale = tip_fetched_at.elapsed() > Duration::from_secs(2);

        if stale || caught_up {
            match rpc.get_block_number().await {
                Ok(tip) => {
                    let new_safe_tip = tip.saturating_sub(confirmation_blocks);
                    stats.current_block.store(tip, Relaxed);

                    if caught_up && new_safe_tip <= cached_safe_tip {
                        // No new blocks since last check — sleep before re-polling
                        sleep(poll_interval).await;
                    }

                    cached_safe_tip = new_safe_tip;
                    tip_fetched_at = Instant::now();
                }
                Err(e) => {
                    error!("[{}] Fetcher: failed to get block number: {}", chain_name, e);
                    sleep(poll_interval).await;
                    continue;
                }
            }
        }

        // 2. Fill JoinSet: retry slots first, then forward slots
        let max_retry_slots = max_concurrent; // same as forward
        let max_total = max_concurrent * 2;   // forward + retry
        let mut retry_filled = 0;

        // 2a. Fill retry slots (up to max_concurrent_fetches)
        while retry_filled < max_retry_slots && join_set.len() < max_total {
            match pending_ranges.pop_front() {
                Some((from, to)) => {
                    retry_filled += 1;
                    let rpc_clone = Arc::clone(&rpc);
                    join_set.spawn(async move {
                        let result = fetch_all_logs(rpc_clone, chain_id, chain_name, from, to).await;
                        (from, to, result)
                    });
                }
                None => break,
            }
        }

        // 2b. Fill forward slots; if caught up, give remaining to retries
        while join_set.len() < max_total {
            let range = if next_from <= cached_safe_tip {
                let to = (next_from + step - 1).min(cached_safe_tip);
                let r = (next_from, to);
                next_from = to + 1;
                Some(r)
            } else {
                pending_ranges.pop_front() // caught up: retries can use forward slots
            };

            match range {
                Some((from, to)) => {
                    let rpc_clone = Arc::clone(&rpc);
                    join_set.spawn(async move {
                        let result = fetch_all_logs(rpc_clone, chain_id, chain_name, from, to).await;
                        (from, to, result)
                    });
                }
                None => break,
            }
        }

        // Update stats after fill
        stats.pending_ranges.store(pending_ranges.len() as u64, Relaxed);
        stats.last_chance_count.store(last_chance_ranges.len() as u64, Relaxed);
        stats.inflight_fetches.store(join_set.len() as u64, Relaxed);

        // 3. If nothing in-flight, we're caught up - loop back (sleep happens in step 1)
        if join_set.is_empty() {
            continue;
        }

        // 4. Wait for any fetch to complete
        let Some(join_result) = join_set.join_next().await else {
            continue;
        };

        match join_result {
            Ok((from, to, Ok(data))) => {
                stats.successful_fetches.fetch_add(1, Relaxed);
                debug!(
                    "[{}] Fetcher: completed blocks {}-{} ({} transfer logs)",
                    chain_name, from, to, data.transfer_logs.len()
                );
                if tx.send(data).await.is_err() {
                    info!("[{}] Fetcher: processor channel closed, shutting down", chain_name);
                    return;
                }
            }
            Ok((from, to, Err(e))) => {
                stats.failed_fetches.fetch_add(1, Relaxed);
                if last_chance_ranges.remove(&(from, to)) {
                    // Last chance with 2x timeout also failed — truly give up
                    stats.timed_out_fetches.fetch_add(1, Relaxed);
                    error!(
                        "[{}] Fetcher: GIVING UP on blocks {}-{} (last chance failed): {}",
                        chain_name, from, to, e
                    );
                    let empty_data = FetchedData {
                        from_block: from,
                        to_block: to,
                        transfer_logs: vec![],
                        fusion_plus_factory_logs: vec![],
                        fusion_plus_escrow_logs: vec![],
                        fusion_logs: vec![],
                        crypto2fiat_logs: vec![],
                    };
                    if tx.send(empty_data).await.is_err() {
                        info!("[{}] Fetcher: processor channel closed, shutting down", chain_name);
                        return;
                    }
                } else if from == to {
                    // Single block — can't split, try last chance with 2x timeout
                    warn!(
                        "[{}] Fetcher: single block {} failed, retrying with 2x timeout: {}",
                        chain_name, from, e
                    );
                    last_chance_ranges.insert((from, to));
                    let rpc_clone = Arc::clone(&slow_rpc);
                    join_set.spawn(async move {
                        let result = fetch_all_logs(rpc_clone, chain_id, chain_name, from, to).await;
                        (from, to, result)
                    });
                } else {
                    // Split range in half and retry both halves
                    let mid = from + (to - from) / 2;
                    warn!(
                        "[{}] Fetcher: error for blocks {}-{}, splitting into {}-{} and {}-{}: {}",
                        chain_name, from, to, from, mid, mid + 1, to, e
                    );
                    pending_ranges.push_back((from, mid));
                    pending_ranges.push_back((mid + 1, to));

                    // Backpressure: cap pending at 10, overflow gets last chance with 2x timeout
                    while pending_ranges.len() > 10 {
                        if let Some((f, t)) = pending_ranges.pop_front() {
                            warn!(
                                "[{}] Fetcher: pending overflow for blocks {}-{}, retrying with 2x timeout",
                                chain_name, f, t
                            );
                            last_chance_ranges.insert((f, t));
                            let rpc_clone = Arc::clone(&slow_rpc);
                            join_set.spawn(async move {
                                let result = fetch_all_logs(rpc_clone, chain_id, chain_name, f, t).await;
                                (f, t, result)
                            });
                        }
                    }
                }
            }
            Err(join_err) => {
                error!("[{}] Fetcher: task panicked: {}", chain_name, join_err);
            }
        }
    }
}

// =============================================================================
// ChainPoller - owns DB, RPC, config; runs processor loop
// =============================================================================

/// Per-chain poller that fetches Transfer events and stores them in PostgreSQL
pub struct ChainPoller {
    network: NetworkConfig,
    rpc: Arc<RpcClient>,
    slow_rpc: Arc<RpcClient>,
    db: Arc<Database>,
    stats: Arc<ChainStats>,
    config: PollerConfig,
    live_config: Arc<LiveConfig>,
    block_timestamp_cache: HashMap<u64, u64>,
}

impl ChainPoller {
    pub fn new(network: NetworkConfig, db: Arc<Database>, stats: Arc<ChainStats>, live_config: Arc<LiveConfig>) -> Self {
        let mut config = PollerConfig::default();
        config.max_blocks_per_query = network.blocks_per_request;
        config.max_concurrent_fetches = network.concurrent_fetches;
        config.channel_capacity = network.concurrent_fetches * 4;
        config.poll_interval_ms = network.poll_interval_ms;
        config.confirmation_blocks = network.confirmation_blocks;
        Self::with_config(network, db, stats, config, live_config)
    }

    pub fn with_config(
        network: NetworkConfig,
        db: Arc<Database>,
        stats: Arc<ChainStats>,
        config: PollerConfig,
        live_config: Arc<LiveConfig>,
    ) -> Self {
        let rpc = Arc::new(RpcClient::new(&network.rpc_url, network.name));
        let slow_rpc = Arc::new(RpcClient::with_config(&network.rpc_url, network.name, 3, 100, 60));

        Self {
            network,
            rpc,
            slow_rpc,
            db,
            stats,
            config,
            live_config,
            block_timestamp_cache: HashMap::new(),
        }
    }

    /// Run the poller with decoupled producer-consumer architecture.
    /// Spawns a fetcher task (producer) and runs a processor loop (consumer).
    pub async fn run(&mut self) {
        info!(
            "[{}] Starting poller (chain_id: {})",
            self.network.name, self.network.chain_id
        );

        // Get starting block from checkpoint
        let last_processed_block = match self.initialize_checkpoint().await {
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

        // Create bounded channel (backpressure)
        let (tx, rx) = mpsc::channel::<FetchedData>(self.config.channel_capacity);

        // Spawn fetcher as independent task (reads config from Arc<LiveConfig>)
        let rpc_clone = Arc::clone(&self.rpc);
        let slow_rpc_clone = Arc::clone(&self.slow_rpc);
        let stats_clone = Arc::clone(&self.stats);
        let live_config_clone = Arc::clone(&self.live_config);
        let chain_id = self.network.chain_id;
        let chain_name = self.network.name;
        let fetcher_handle = tokio::spawn(async move {
            fetcher_loop(
                rpc_clone, slow_rpc_clone, chain_id, chain_name,
                last_processed_block, live_config_clone, tx, stats_clone,
            ).await;
        });

        // Run processor on current task (needs &mut self for timestamp cache)
        self.processor_loop(rx, last_processed_block).await;

        // If processor exits, abort fetcher
        fetcher_handle.abort();
        info!("[{}] Poller stopped", self.network.name);
    }

    // =========================================================================
    // Processor Loop (Consumer)
    // =========================================================================

    /// Receives fetched data from channel, buffers in BTreeMap, processes in
    /// contiguous block order, and advances checkpoint only when there are no gaps.
    /// Decoupled processor: inserts data for ANY available range immediately,
    /// then advances the checkpoint only through contiguous inserted ranges.
    /// This prevents out-of-order ranges from blocking the insertion pipeline.
    async fn processor_loop(
        &mut self,
        mut rx: mpsc::Receiver<FetchedData>,
        start_from: u64,
    ) {
        let mut buffer: BTreeMap<u64, FetchedData> = BTreeMap::new();
        let mut checkpoint_next: u64 = start_from + 1;
        // Tracks ranges that have been inserted to DB but not yet checkpointed
        // Key: from_block, Value: to_block
        let mut inserted: BTreeMap<u64, u64> = BTreeMap::new();
        let mut retry_counts: HashMap<u64, u32> = HashMap::new();

        info!(
            "[{}] Processor started, checkpoint frontier: {}",
            self.network.name, checkpoint_next
        );

        loop {
            // 1. Drain all immediately available items from channel into buffer
            let mut channel_disconnected = false;
            loop {
                match rx.try_recv() {
                    Ok(data) => {
                        buffer.insert(data.from_block, data);
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        channel_disconnected = true;
                        break;
                    }
                }
            }

            // 2. Process ALL available ranges in buffer (not just the next contiguous one)
            let mut processed_any = false;
            let keys: Vec<u64> = buffer.keys().cloned().collect();
            for key in keys {
                let data = buffer.remove(&key).unwrap();

                match self.process_fetched_data(&data).await {
                    Ok(events) => {
                        if events > 0 {
                            debug!(
                                "[{}] Inserted {} events for blocks {}-{}",
                                self.network.name, events, data.from_block, data.to_block
                            );
                        }
                        // Data inserted successfully — track for checkpoint advancement
                        inserted.insert(data.from_block, data.to_block);
                        self.stats.total_transfers.fetch_add(events as u64, Relaxed);
                        retry_counts.remove(&data.from_block);
                        processed_any = true;
                        // FetchedData is dropped here — we only keep the lightweight (from, to) tracking
                    }
                    Err(e) => {
                        let from = data.from_block;
                        let to = data.to_block;
                        let attempts = retry_counts.entry(from).or_insert(0);
                        *attempts += 1;

                        if *attempts > 10 {
                            // Give up — mark as "inserted" so checkpoint can advance past it
                            error!(
                                "[{}] GIVING UP on blocks {}-{} after {} attempts: {}",
                                self.network.name, from, to, attempts, e
                            );
                            inserted.insert(from, to);
                            retry_counts.remove(&from);
                            processed_any = true;
                        } else {
                            error!(
                                "[{}] Process error for blocks {}-{} (attempt {}): {}",
                                self.network.name, from, to, attempts, e
                            );
                            // Re-insert data for retry on next iteration
                            buffer.insert(from, data);
                        }
                    }
                }
            }

            // 3. Advance checkpoint through contiguous inserted ranges
            while let Some(&to_block) = inserted.get(&checkpoint_next) {
                match self.db.set_checkpoint(self.network.chain_id, to_block).await {
                    Ok(_) => {
                        let blocks_in_range = to_block - checkpoint_next + 1;
                        self.stats.checkpoint_block.store(to_block, Relaxed);
                        self.stats.blocks_processed.fetch_add(blocks_in_range, Relaxed);
                        inserted.remove(&checkpoint_next);
                        checkpoint_next = to_block + 1;
                    }
                    Err(e) => {
                        error!(
                            "[{}] Checkpoint error at {}: {}",
                            self.network.name, to_block, e
                        );
                        sleep(Duration::from_secs(1)).await;
                        break;
                    }
                }
            }

            // 4. Clean timestamp cache based on checkpoint frontier (safe cutoff)
            self.cleanup_timestamp_cache(checkpoint_next.saturating_sub(1));

            // 5. Update buffer stat (unprocessed buffer + pending checkpoint ranges)
            self.stats.buffer_size.store(
                (buffer.len() + inserted.len()) as u64,
                Relaxed,
            );

            // 6. If channel disconnected, process remaining and exit
            if channel_disconnected {
                // Insert any remaining buffer entries
                let remaining_keys: Vec<u64> = buffer.keys().cloned().collect();
                for key in remaining_keys {
                    if let Some(data) = buffer.remove(&key) {
                        if let Ok(events) = self.process_fetched_data(&data).await {
                            inserted.insert(data.from_block, data.to_block);
                            self.stats.total_transfers.fetch_add(events as u64, Relaxed);
                        }
                    }
                }
                // Final checkpoint advancement
                while let Some(&to_block) = inserted.get(&checkpoint_next) {
                    match self.db.set_checkpoint(self.network.chain_id, to_block).await {
                        Ok(_) => {
                            self.stats.checkpoint_block.store(to_block, Relaxed);
                            inserted.remove(&checkpoint_next);
                            checkpoint_next = to_block + 1;
                        }
                        Err(_) => break,
                    }
                }
                info!(
                    "[{}] Fetcher disconnected, processor exiting (pending checkpoint: {})",
                    self.network.name, inserted.len()
                );
                return;
            }

            // 7. If nothing processed and buffer empty, blocking recv for new data
            if !processed_any && buffer.is_empty() {
                match rx.recv().await {
                    Some(data) => {
                        buffer.insert(data.from_block, data);
                    }
                    None => {
                        info!(
                            "[{}] Fetcher channel closed, processor exiting",
                            self.network.name
                        );
                        return;
                    }
                }
            }
        }
    }

    // =========================================================================
    // Checkpoint Initialization
    // =========================================================================

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
    // Data Processing (DB insert + fusion processing)
    // =========================================================================

    /// Process previously fetched data: build transfers, insert to DB,
    /// process fusion/c2f events. Checkpoint is handled by the caller.
    async fn process_fetched_data(
        &mut self,
        data: &FetchedData,
    ) -> Result<usize, String> {
        // =====================================================================
        // Pre-fetch all unique block timestamps in parallel
        // =====================================================================
        {
            let mut needed_blocks: HashSet<u64> = HashSet::new();
            for log in &data.transfer_logs {
                needed_blocks.insert(log.block_number_u64());
            }
            for log in &data.fusion_plus_factory_logs {
                needed_blocks.insert(log.block_number_u64());
            }
            for log in &data.fusion_plus_escrow_logs {
                needed_blocks.insert(log.block_number_u64());
            }
            for log in &data.fusion_logs {
                needed_blocks.insert(log.block_number_u64());
            }
            for log in &data.crypto2fiat_logs {
                needed_blocks.insert(log.block_number_u64());
            }

            // Filter out already-cached block numbers
            let uncached: Vec<u64> = needed_blocks
                .into_iter()
                .filter(|b| !self.block_timestamp_cache.contains_key(b))
                .collect();

            if !uncached.is_empty() {
                let mut futs = Vec::with_capacity(uncached.len());
                for block_num in &uncached {
                    let rpc = Arc::clone(&self.rpc);
                    let bn = *block_num;
                    futs.push(async move {
                        let result = rpc.get_block(bn).await;
                        (bn, result)
                    });
                }

                let results = futures::future::join_all(futs).await;

                for (block_num, result) in results {
                    match result {
                        Ok(block) => {
                            self.block_timestamp_cache.insert(block_num, block.timestamp_u64());
                        }
                        Err(e) => {
                            return Err(format!("Failed to get block {}: {}", block_num, e));
                        }
                    }
                }
            }
        }

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
        let batch_len = transfers.len();
        let insert_start = std::time::Instant::now();

        let copy_threshold = self.live_config.copy_threshold.load(Relaxed) as usize;
        let inserted = if !transfers.is_empty() {
            self.db
                .insert_transfers_batch(self.network.chain_id, &transfers, copy_threshold)
                .await
                .map_err(|e| format!("DB error: {}", e))?
        } else {
            0
        };

        let insert_ms = insert_start.elapsed().as_millis() as u64;
        self.stats.last_insert_time_ms.store(insert_ms, Relaxed);
        self.stats.last_batch_size.store(batch_len as u64, Relaxed);

        // =====================================================================
        // Process fusion events (insert swap records, no UPDATE needed)
        // Must run AFTER insert_transfers_batch (fusion needs transfers in DB)
        // =====================================================================
        let fusion_plus_events = self.process_fusion_plus_logs(
            &data.fusion_plus_factory_logs, &data.fusion_plus_escrow_logs
        ).await?;
        let fusion_events = self.process_fusion_logs(&data.fusion_logs).await?;
        let crypto2fiat_events = self.process_crypto2fiat_logs(&data.crypto2fiat_logs).await?;

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
