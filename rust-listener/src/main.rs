#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod adapter;
mod bitcoin_adapter;
mod bitcoin_rpc;
mod solana_adapter;
mod solana_rpc;
mod config;
mod db;
mod evm_adapter;
mod fusion;
mod poller;
mod rpc;
mod types;

use crate::adapter::ChainAdapter;
use crate::bitcoin_adapter::BitcoinAdapter;
use crate::bitcoin_rpc::BitcoinRpcClient;
use crate::solana_adapter::SolanaAdapter;
use crate::solana_rpc::SolanaRpcClient;
use crate::config::{get_database_url, get_ttl_secs, load_networks};
use crate::db::Database;
use crate::evm_adapter::EvmAdapter;
use crate::poller::ChainPoller;
use crate::rpc::RpcClient;
use crate::types::{ChainStats, ChainType, LiveConfig};
use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::sleep;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    // Initialize logging
    let log_level = std::env::var("LOG_LEVEL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(Level::INFO);

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    info!("Starting Rust Blockchain Listener");

    // Load configuration
    let database_url = get_database_url();
    let ttl_secs = get_ttl_secs();
    let networks = load_networks();

    info!("Database: PostgreSQL");
    info!("TTL: {} seconds ({} minutes)", ttl_secs, ttl_secs / 60);
    info!("Networks: {} chains configured", networks.len());

    // Get chain IDs from networks
    let chain_ids: Vec<u32> = networks.iter().map(|n| n.chain_id).collect();
    info!("Chain IDs: {:?}", chain_ids);

    // Open PostgreSQL database connection pool
    let db = match Database::new(&database_url, &chain_ids).await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to connect to PostgreSQL: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "PostgreSQL database connected. Schema auto-created for {} chains.",
        chain_ids.len()
    );

    // Spawn cleanup task
    let db_cleanup = Arc::clone(&db);
    let cleanup_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await;

            // Clean up old data from all tables
            match db_cleanup.cleanup_all(ttl_secs).await {
                Ok(stats) => {
                    let total_deleted = stats.transfers_deleted
                        + stats.fusion_plus_deleted
                        + stats.fusion_deleted
                        + stats.crypto2fiat_deleted
                        + stats.metrics_deleted;
                    if total_deleted > 0 {
                        info!(
                            "Cleanup: removed {} transfers, {} Fusion+ swaps, {} Fusion swaps, {} Crypto2Fiat events, {} metrics snapshots",
                            stats.transfers_deleted,
                            stats.fusion_plus_deleted,
                            stats.fusion_deleted,
                            stats.crypto2fiat_deleted,
                            stats.metrics_deleted
                        );
                    }
                }
                Err(e) => {
                    warn!("Cleanup error: {}", e);
                }
            }

            // Log stats every cleanup cycle
            let transfer_count = db_cleanup.get_total_transfer_count().await.unwrap_or(0);
            let fusion_plus_count = db_cleanup.get_fusion_plus_count().await.unwrap_or(0);
            let fusion_count = db_cleanup.get_fusion_swap_count().await.unwrap_or(0);
            let crypto2fiat_count = db_cleanup.get_crypto2fiat_count().await.unwrap_or(0);
            info!(
                "Database stats: {} transfers, {} Fusion+ swaps, {} Fusion swaps, {} Crypto2Fiat events",
                transfer_count, fusion_plus_count, fusion_count, crypto2fiat_count
            );
        }
    });

    // Spawn poller for each chain (with per-chain stats + live config)
    let mut poller_handles = Vec::new();
    let mut all_stats: Vec<Arc<ChainStats>> = Vec::new();
    let mut all_live_configs: Vec<(u32, Arc<LiveConfig>)> = Vec::new();

    for network in networks {
        let db_clone = Arc::clone(&db);
        let chain_name = network.name.to_string();
        let stats = Arc::new(ChainStats::new(network.chain_id, network.name));
        let live_config = Arc::new(LiveConfig::from_network(&network));
        all_stats.push(Arc::clone(&stats));
        all_live_configs.push((network.chain_id, Arc::clone(&live_config)));

        // Construct protocol adapter based on chain type
        let (adapter, slow_adapter): (Arc<dyn ChainAdapter>, Arc<dyn ChainAdapter>) =
            match network.chain_type {
                ChainType::Evm => {
                    let rpc = Arc::new(RpcClient::new(&network.rpc_url, network.name));
                    let slow_rpc = Arc::new(RpcClient::with_config(
                        &network.rpc_url,
                        network.name,
                        3,
                        100,
                        60,
                    ));
                    (
                        Arc::new(EvmAdapter::new(rpc, network.chain_id, network.name)),
                        Arc::new(EvmAdapter::new(slow_rpc, network.chain_id, network.name)),
                    )
                }
                ChainType::Bitcoin => {
                    let btc_rpc = Arc::new(BitcoinRpcClient::new(
                        &network.rpc_url,
                        network.rpc_user.as_deref().unwrap_or(""),
                        network.rpc_password.as_deref().unwrap_or(""),
                        network.name,
                    ));
                    let adapter: Arc<dyn ChainAdapter> = Arc::new(BitcoinAdapter::new(
                        Arc::clone(&btc_rpc),
                        network.chain_id,
                        network.name,
                    ));
                    // Bitcoin uses same adapter for both (no fast/slow distinction)
                    (Arc::clone(&adapter), adapter)
                }
                ChainType::Solana => {
                    let sol_rpc = Arc::new(SolanaRpcClient::new(
                        &network.rpc_url,
                        network.name,
                    ));
                    let adapter: Arc<dyn ChainAdapter> = Arc::new(SolanaAdapter::new(
                        Arc::clone(&sol_rpc),
                        network.chain_id,
                        network.name,
                    ));
                    (Arc::clone(&adapter), adapter)
                }
            };

        let handle = tokio::spawn(async move {
            let mut poller =
                ChainPoller::new(network, adapter, slow_adapter, db_clone, stats, live_config);
            poller.run().await;
        });

        info!("Spawned poller for {}", chain_name);
        poller_handles.push(handle);
    }

    // Spawn config watcher task (reads DB overrides every 5s, updates LiveConfig atomics)
    let db_config_watcher = Arc::clone(&db);
    let configs_for_watcher = all_live_configs.clone();
    let config_watcher_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            match db_config_watcher.get_all_config_overrides().await {
                Ok(overrides) => {
                    let override_map: HashMap<i32, _> =
                        overrides.into_iter().map(|o| (o.chain_id, o)).collect();
                    for (chain_id, live_config) in &configs_for_watcher {
                        if let Some(ov) = override_map.get(&(*chain_id as i32)) {
                            // Apply overrides (only non-NULL values)
                            if let Some(v) = ov.blocks_per_request {
                                live_config.max_blocks_per_query.store(v as u64, Relaxed);
                            } else {
                                live_config.max_blocks_per_query.store(live_config.default_blocks_per_query, Relaxed);
                            }
                            if let Some(v) = ov.concurrent_fetches {
                                live_config.max_concurrent_fetches.store(v as usize, Relaxed);
                            } else {
                                live_config.max_concurrent_fetches.store(live_config.default_concurrent_fetches, Relaxed);
                            }
                            if let Some(v) = ov.poll_interval_ms {
                                live_config.poll_interval_ms.store(v as u64, Relaxed);
                            } else {
                                live_config.poll_interval_ms.store(live_config.default_poll_interval_ms, Relaxed);
                            }
                            if let Some(v) = ov.confirmation_blocks {
                                live_config.confirmation_blocks.store(v as u64, Relaxed);
                            } else {
                                live_config.confirmation_blocks.store(live_config.default_confirmation_blocks, Relaxed);
                            }
                            if let Some(v) = ov.copy_threshold {
                                live_config.copy_threshold.store(v as u64, Relaxed);
                            } else {
                                live_config.copy_threshold.store(live_config.default_copy_threshold, Relaxed);
                            }
                            if let Some(v) = ov.concurrent_inserts {
                                live_config.max_concurrent_inserts.store(v as usize, Relaxed);
                            } else {
                                live_config.max_concurrent_inserts.store(live_config.default_concurrent_inserts, Relaxed);
                            }
                        } else {
                            // No override for this chain — reset to defaults
                            live_config.max_blocks_per_query.store(live_config.default_blocks_per_query, Relaxed);
                            live_config.max_concurrent_fetches.store(live_config.default_concurrent_fetches, Relaxed);
                            live_config.poll_interval_ms.store(live_config.default_poll_interval_ms, Relaxed);
                            live_config.confirmation_blocks.store(live_config.default_confirmation_blocks, Relaxed);
                            live_config.copy_threshold.store(live_config.default_copy_threshold, Relaxed);
                            live_config.max_concurrent_inserts.store(live_config.default_concurrent_inserts, Relaxed);
                        }
                    }
                }
                Err(e) => warn!("Config watcher error: {}", e),
            }
        }
    });

    // Spawn stats writer task (every 1 second, writes all chain stats to DB + metrics history)
    // Uses a SINGLE pool connection per iteration to avoid pool contention with chain inserters
    let db_stats = Arc::clone(&db);
    let stats_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(1)).await;

            // Acquire one connection for the entire iteration
            let client = match db_stats.get_client().await {
                Ok(c) => c,
                Err(e) => {
                    warn!("Stats writer: failed to get DB client: {}", e);
                    continue;
                }
            };

            for stats in &all_stats {
                let current = stats.current_block.load(Relaxed);
                let checkpoint = stats.checkpoint_block.load(Relaxed);
                let insert_time = stats.last_insert_time_ms.load(Relaxed);
                let batch = stats.last_batch_size.load(Relaxed);
                let buf = stats.buffer_size.load(Relaxed);
                let events = stats.total_transfers.load(Relaxed);
                let fetch_time = stats.last_fetch_time_ms.load(Relaxed);
                let pool_wait = stats.last_pool_wait_ms.load(Relaxed);
                let rows_inserted = stats.last_rows_inserted.load(Relaxed);
                let commit_ms = stats.last_commit_ms.load(Relaxed);
                let insert_method = stats.last_insert_method.load(Relaxed);
                let copy_threshold = stats.last_copy_threshold.load(Relaxed);
                let cum_insert_ms = stats.cumulative_insert_ms.load(Relaxed);
                let cum_inserts = stats.cumulative_inserts.load(Relaxed);
                let avg_insert_ms = if cum_inserts > 0 { cum_insert_ms / cum_inserts } else { 0 };

                if let Err(e) = Database::upsert_listener_stats_on(
                    &client,
                    stats.chain_id,
                    stats.chain_name,
                    current,
                    checkpoint,
                    stats.pending_ranges.load(Relaxed),
                    stats.last_chance_count.load(Relaxed),
                    stats.inflight_fetches.load(Relaxed),
                    stats.successful_fetches.load(Relaxed),
                    stats.failed_fetches.load(Relaxed),
                    stats.timed_out_fetches.load(Relaxed),
                    stats.blocks_processed.load(Relaxed),
                    events,
                    buf,
                    insert_time,
                    batch,
                    fetch_time,
                    pool_wait,
                    rows_inserted,
                    commit_ms,
                    insert_method,
                    copy_threshold,
                    avg_insert_ms,
                    cum_inserts,
                ).await {
                    warn!("Failed to write stats for chain {}: {}", stats.chain_id, e);
                }

                // Record metrics history for charts
                let _ = Database::insert_metrics_snapshot_on(
                    &client,
                    stats.chain_id,
                    insert_time,
                    batch,
                    buf,
                    current.saturating_sub(checkpoint),
                    events,
                    fetch_time,
                    pool_wait,
                    commit_ms,
                    rows_inserted,
                ).await;
            }
        }
    });

    info!("All {} pollers started", poller_handles.len());
    info!("Press Ctrl+C to stop");

    // Wait for shutdown signal
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Shutdown signal received");
        }
        Err(e) => {
            error!("Failed to listen for shutdown: {}", e);
        }
    }

    // Graceful shutdown
    info!("Shutting down...");

    // Abort all tasks
    for handle in poller_handles {
        handle.abort();
    }
    cleanup_handle.abort();
    stats_handle.abort();
    config_watcher_handle.abort();

    info!("Shutdown complete");
}
