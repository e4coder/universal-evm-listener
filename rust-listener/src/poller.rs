use crate::adapter::ChainAdapter;
use crate::db::Database;
use crate::types::{
    ChainStats, DecodedBlockRange, FpAction, FusionAction, LiveConfig, NetworkConfig,
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

// =============================================================================
// Generic DB Insertion (protocol-agnostic)
// =============================================================================

/// Insert decoded events from a DecodedBlockRange into the database.
/// All DB operations wrapped in a single transaction. Standalone for tokio::spawn.
async fn insert_decoded_range(
    db: Arc<Database>,
    chain_id: u32,
    chain_name: &'static str,
    live_config: Arc<LiveConfig>,
    stats: Arc<ChainStats>,
    data: &DecodedBlockRange,
) -> Result<usize, String> {
    let insert_start = std::time::Instant::now();
    let batch_len = data.transfers.len();

    let mut client = db
        .get_client()
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    let copy_threshold = live_config.copy_threshold.load(Relaxed) as usize;
    let mut total = 0;

    // 1. Insert transfers
    if !data.transfers.is_empty() {
        total += Database::insert_transfers_batch_on(&tx, chain_id, &data.transfers, copy_threshold)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
    }

    let insert_ms = insert_start.elapsed().as_millis() as u64;
    stats.last_insert_time_ms.store(insert_ms, Relaxed);
    stats.last_batch_size.store(batch_len as u64, Relaxed);

    // 2. Fusion+ actions (pattern match on enum — no protocol-specific decoding)
    let mut fp_count = 0;
    for action in &data.fusion_plus_actions {
        match action {
            FpAction::SrcEscrowCreated { swap } => {
                if let Err(e) = Database::insert_fusion_plus_swap_on(&tx, swap).await {
                    warn!("[{}] Failed to insert Fusion+ SrcEscrowCreated: {}", chain_name, e);
                } else {
                    fp_count += 1;
                }
            }
            FpAction::DstEscrowCreated {
                order_hash,
                data: dst,
                chain_id: cid,
                tx_hash,
                block_number,
                block_timestamp,
                log_index,
                escrow_address,
            } => {
                match Database::update_fusion_plus_dst_on(
                    &tx,
                    order_hash,
                    dst,
                    *cid,
                    tx_hash,
                    *block_number,
                    *block_timestamp,
                    *log_index,
                    Some(escrow_address),
                )
                .await
                {
                    Ok(updated) => {
                        if !updated {
                            debug!(
                                "[{}] Fusion+ DstEscrow created for unknown order: {}",
                                chain_name, order_hash
                            );
                        }
                        fp_count += 1;
                    }
                    Err(e) => {
                        warn!("[{}] Failed to update Fusion+ DstEscrowCreated: {}", chain_name, e);
                    }
                }
            }
            FpAction::EscrowWithdrawal {
                secret,
                hashlock,
                chain_id: cid,
                tx_hash,
                block_number,
                block_timestamp,
                log_index,
                ..
            } => {
                // DB lookup for existing swap by hashlock (cross-chain, may be from previous range)
                if let Ok(Some(swap)) =
                    Database::get_fusion_plus_swap_by_hashlock_on(&tx, hashlock).await
                {
                    let is_src = swap.src_chain_id == *cid;
                    match Database::update_fusion_plus_withdrawal_by_hashlock_on(
                        &tx,
                        hashlock,
                        *cid,
                        is_src,
                        secret,
                        tx_hash,
                        *block_number,
                        *block_timestamp,
                        *log_index,
                    )
                    .await
                    {
                        Ok(updated) => {
                            if updated {
                                let side = if is_src { "source" } else { "destination" };
                                info!(
                                    "[{}] Fusion+ {} withdrawal: order_hash={} secret={} tx={}",
                                    chain_name, side, swap.order_hash, secret, tx_hash
                                );
                            }
                            fp_count += 1;
                        }
                        Err(e) => {
                            warn!("[{}] Failed to update Fusion+ withdrawal: {}", chain_name, e);
                        }
                    }
                }
            }
            FpAction::EscrowCancelled { .. } => {
                fp_count += 1;
            }
        }
    }
    if fp_count > 0 {
        info!("[{}] Processed {} Fusion+ events", chain_name, fp_count);
    }

    // 3. Fusion single-chain actions
    let mut fs_count = 0;
    for action in &data.fusion_actions {
        let swap = match action {
            FusionAction::OrderFilled { swap } | FusionAction::OrderCancelled { swap } => swap,
        };
        if let Err(e) = Database::insert_fusion_swap_on(&tx, swap).await {
            warn!("[{}] Failed to insert Fusion swap: {}", chain_name, e);
        } else {
            fs_count += 1;
        }
    }
    if fs_count > 0 {
        info!("[{}] Processed {} Fusion events", chain_name, fs_count);
    }

    // 4. Crypto2Fiat events
    let mut c2f_count = 0;
    for event in &data.crypto2fiat_events {
        if let Err(e) = Database::insert_crypto2fiat_event_on(&tx, event).await {
            warn!("[{}] Failed to insert Crypto2Fiat event: {}", chain_name, e);
        } else {
            c2f_count += 1;
        }
    }
    if c2f_count > 0 {
        info!("[{}] Processed {} Crypto2Fiat events", chain_name, c2f_count);
    }

    tx.commit()
        .await
        .map_err(|e| format!("DB commit error: {}", e))?;

    total += fp_count + fs_count + c2f_count;
    Ok(total)
}

// =============================================================================
// Fetcher Loop (Producer) - standalone function, spawned as independent task
// =============================================================================

/// Continuously fetches block ranges and sends decoded results to the processor via channel.
/// Uses the ChainAdapter trait for protocol-agnostic block fetching + decoding.
async fn fetcher_loop(
    adapter: Arc<dyn ChainAdapter>,
    slow_adapter: Arc<dyn ChainAdapter>,
    _chain_id: u32,
    chain_name: &'static str,
    start_from: u64,
    live_config: Arc<LiveConfig>,
    tx: mpsc::Sender<DecodedBlockRange>,
    stats: Arc<ChainStats>,
) {
    let mut next_from = start_from + 1;
    let mut join_set: JoinSet<(u64, u64, Result<DecodedBlockRange, String>, u64)> = JoinSet::new();
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
            match adapter.get_block_number().await {
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
                    error!(
                        "[{}] Fetcher: failed to get block number: {}",
                        chain_name, e
                    );
                    sleep(poll_interval).await;
                    continue;
                }
            }
        }

        // 2. Fill JoinSet: retry slots first, then forward slots
        let max_retry_slots = max_concurrent; // same as forward
        let max_total = max_concurrent * 2; // forward + retry
        let mut retry_filled = 0;

        // 2a. Fill retry slots (up to max_concurrent_fetches)
        while retry_filled < max_retry_slots && join_set.len() < max_total {
            match pending_ranges.pop_front() {
                Some((from, to)) => {
                    retry_filled += 1;
                    let adapter_clone = Arc::clone(&adapter);
                    join_set.spawn(async move {
                        let start = Instant::now();
                        let result = adapter_clone.fetch_decoded(from, to).await;
                        (from, to, result, start.elapsed().as_millis() as u64)
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
                    let adapter_clone = Arc::clone(&adapter);
                    join_set.spawn(async move {
                        let start = Instant::now();
                        let result = adapter_clone.fetch_decoded(from, to).await;
                        (from, to, result, start.elapsed().as_millis() as u64)
                    });
                }
                None => break,
            }
        }

        // Update stats after fill
        stats
            .pending_ranges
            .store(pending_ranges.len() as u64, Relaxed);
        stats
            .last_chance_count
            .store(last_chance_ranges.len() as u64, Relaxed);
        stats
            .inflight_fetches
            .store(join_set.len() as u64, Relaxed);

        // 3. If nothing in-flight, we're caught up - loop back (sleep happens in step 1)
        if join_set.is_empty() {
            continue;
        }

        // 4. Wait for any fetch to complete
        let Some(join_result) = join_set.join_next().await else {
            continue;
        };

        match join_result {
            Ok((from, to, Ok(data), elapsed_ms)) => {
                stats.successful_fetches.fetch_add(1, Relaxed);
                stats.last_fetch_time_ms.store(elapsed_ms, Relaxed);
                debug!(
                    "[{}] Fetcher: completed blocks {}-{} ({} transfers)",
                    chain_name,
                    from,
                    to,
                    data.transfers.len()
                );
                if tx.send(data).await.is_err() {
                    info!(
                        "[{}] Fetcher: processor channel closed, shutting down",
                        chain_name
                    );
                    return;
                }
            }
            Ok((from, to, Err(e), _elapsed_ms)) => {
                stats.failed_fetches.fetch_add(1, Relaxed);
                if last_chance_ranges.remove(&(from, to)) {
                    // Last chance with slow adapter also failed — truly give up
                    stats.timed_out_fetches.fetch_add(1, Relaxed);
                    error!(
                        "[{}] Fetcher: GIVING UP on blocks {}-{} (last chance failed): {}",
                        chain_name, from, to, e
                    );
                    let empty_data = DecodedBlockRange {
                        from_block: from,
                        to_block: to,
                        transfers: vec![],
                        fusion_plus_actions: vec![],
                        fusion_actions: vec![],
                        crypto2fiat_events: vec![],
                    };
                    if tx.send(empty_data).await.is_err() {
                        info!(
                            "[{}] Fetcher: processor channel closed, shutting down",
                            chain_name
                        );
                        return;
                    }
                } else if from == to {
                    // Single block — can't split, try last chance with slow adapter
                    warn!(
                        "[{}] Fetcher: single block {} failed, retrying with slow adapter: {}",
                        chain_name, from, e
                    );
                    last_chance_ranges.insert((from, to));
                    let adapter_clone = Arc::clone(&slow_adapter);
                    join_set.spawn(async move {
                        let start = Instant::now();
                        let result = adapter_clone.fetch_decoded(from, to).await;
                        (from, to, result, start.elapsed().as_millis() as u64)
                    });
                } else {
                    // Split range in half and retry both halves
                    let mid = from + (to - from) / 2;
                    warn!(
                        "[{}] Fetcher: error for blocks {}-{}, splitting into {}-{} and {}-{}: {}",
                        chain_name,
                        from,
                        to,
                        from,
                        mid,
                        mid + 1,
                        to,
                        e
                    );
                    pending_ranges.push_back((from, mid));
                    pending_ranges.push_back((mid + 1, to));

                    // Backpressure: cap pending at 10, overflow gets last chance with slow adapter
                    while pending_ranges.len() > 10 {
                        if let Some((f, t)) = pending_ranges.pop_front() {
                            warn!(
                                "[{}] Fetcher: pending overflow for blocks {}-{}, retrying with slow adapter",
                                chain_name, f, t
                            );
                            last_chance_ranges.insert((f, t));
                            let adapter_clone = Arc::clone(&slow_adapter);
                            join_set.spawn(async move {
                                let start = Instant::now();
                                let result = adapter_clone.fetch_decoded(f, t).await;
                                (f, t, result, start.elapsed().as_millis() as u64)
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
// ChainPoller - owns adapter, DB, config; runs processor loop
// =============================================================================

/// Per-chain poller that uses a ChainAdapter to fetch decoded events
/// and stores them in PostgreSQL via the generic insert_decoded_range().
pub struct ChainPoller {
    network: NetworkConfig,
    adapter: Arc<dyn ChainAdapter>,
    slow_adapter: Arc<dyn ChainAdapter>,
    db: Arc<Database>,
    stats: Arc<ChainStats>,
    config: PollerConfig,
    live_config: Arc<LiveConfig>,
}

impl ChainPoller {
    pub fn new(
        network: NetworkConfig,
        adapter: Arc<dyn ChainAdapter>,
        slow_adapter: Arc<dyn ChainAdapter>,
        db: Arc<Database>,
        stats: Arc<ChainStats>,
        live_config: Arc<LiveConfig>,
    ) -> Self {
        let mut config = PollerConfig::default();
        config.max_blocks_per_query = network.blocks_per_request;
        config.max_concurrent_fetches = network.concurrent_fetches;
        config.channel_capacity = network.concurrent_fetches * 4;
        config.poll_interval_ms = network.poll_interval_ms;
        config.confirmation_blocks = network.confirmation_blocks;
        Self {
            network,
            adapter,
            slow_adapter,
            db,
            stats,
            config,
            live_config,
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
        let (tx, rx) = mpsc::channel::<DecodedBlockRange>(self.config.channel_capacity);

        // Spawn fetcher as independent task
        let adapter_clone = Arc::clone(&self.adapter);
        let slow_adapter_clone = Arc::clone(&self.slow_adapter);
        let stats_clone = Arc::clone(&self.stats);
        let live_config_clone = Arc::clone(&self.live_config);
        let chain_id = self.network.chain_id;
        let chain_name = self.network.name;
        let fetcher_handle = tokio::spawn(async move {
            fetcher_loop(
                adapter_clone,
                slow_adapter_clone,
                chain_id,
                chain_name,
                last_processed_block,
                live_config_clone,
                tx,
                stats_clone,
            )
            .await;
        });

        // Run processor on current task
        self.processor_loop(rx, last_processed_block).await;

        // If processor exits, abort fetcher
        fetcher_handle.abort();
        info!("[{}] Poller stopped", self.network.name);
    }

    // =========================================================================
    // Processor Loop (Consumer)
    // =========================================================================

    /// Receives decoded data from channel, buffers in BTreeMap, processes ranges
    /// in parallel via insert_decoded_range(), and advances checkpoint only
    /// through contiguous inserted ranges.
    async fn processor_loop(
        &mut self,
        mut rx: mpsc::Receiver<DecodedBlockRange>,
        start_from: u64,
    ) {
        let mut buffer: BTreeMap<u64, DecodedBlockRange> = BTreeMap::new();
        let mut checkpoint_next: u64 = start_from + 1;
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

            // 2. Process ALL available ranges in parallel
            let mut processed_any = false;
            let keys: Vec<u64> = buffer.keys().cloned().collect();

            if !keys.is_empty() {
                let max_parallel = self
                    .live_config
                    .max_concurrent_inserts
                    .load(Relaxed)
                    .max(1);

                // Process in chunks of max_parallel
                for chunk in keys.chunks(max_parallel) {
                    let mut join_set: JoinSet<(
                        u64,
                        DecodedBlockRange,
                        Result<usize, String>,
                    )> = JoinSet::new();

                    for &key in chunk {
                        if let Some(data) = buffer.remove(&key) {
                            let db = Arc::clone(&self.db);
                            let stats = Arc::clone(&self.stats);
                            let lc = Arc::clone(&self.live_config);
                            let cid = self.network.chain_id;
                            let cname = self.network.name;

                            join_set.spawn(async move {
                                let result =
                                    insert_decoded_range(db, cid, cname, lc, stats, &data).await;
                                (key, data, result)
                            });
                        }
                    }

                    // Collect results
                    while let Some(Ok((_key, data, result))) = join_set.join_next().await {
                        match result {
                            Ok(events) => {
                                if events > 0 {
                                    debug!(
                                        "[{}] Inserted {} events for blocks {}-{}",
                                        self.network.name, events, data.from_block, data.to_block
                                    );
                                }
                                inserted.insert(data.from_block, data.to_block);
                                self.stats
                                    .total_transfers
                                    .fetch_add(events as u64, Relaxed);
                                retry_counts.remove(&data.from_block);
                                processed_any = true;
                            }
                            Err(e) => {
                                let from = data.from_block;
                                let to = data.to_block;
                                let attempts = retry_counts.entry(from).or_insert(0);
                                *attempts += 1;

                                if *attempts > 10 {
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
                                    buffer.insert(from, data);
                                }
                            }
                        }
                    }
                }
            }

            // 3. Advance checkpoint through contiguous inserted ranges
            while let Some(&to_block) = inserted.get(&checkpoint_next) {
                match self
                    .db
                    .set_checkpoint(self.network.chain_id, to_block)
                    .await
                {
                    Ok(_) => {
                        let blocks_in_range = to_block - checkpoint_next + 1;
                        self.stats.checkpoint_block.store(to_block, Relaxed);
                        self.stats
                            .blocks_processed
                            .fetch_add(blocks_in_range, Relaxed);
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

            // 4. Update buffer stat
            self.stats
                .buffer_size
                .store((buffer.len() + inserted.len()) as u64, Relaxed);

            // 5. If channel disconnected, process remaining and exit
            if channel_disconnected {
                let remaining_keys: Vec<u64> = buffer.keys().cloned().collect();
                for key in remaining_keys {
                    if let Some(data) = buffer.remove(&key) {
                        if let Ok(events) = insert_decoded_range(
                            Arc::clone(&self.db),
                            self.network.chain_id,
                            self.network.name,
                            Arc::clone(&self.live_config),
                            Arc::clone(&self.stats),
                            &data,
                        )
                        .await
                        {
                            inserted.insert(data.from_block, data.to_block);
                            self.stats
                                .total_transfers
                                .fetch_add(events as u64, Relaxed);
                        }
                    }
                }
                // Final checkpoint advancement
                while let Some(&to_block) = inserted.get(&checkpoint_next) {
                    match self
                        .db
                        .set_checkpoint(self.network.chain_id, to_block)
                        .await
                    {
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
                    self.network.name,
                    inserted.len()
                );
                return;
            }

            // 6. If nothing processed and buffer empty, blocking recv for new data
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
        // Get current block from chain via adapter
        let current_block = self
            .adapter
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
}
