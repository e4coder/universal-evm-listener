mod config;
mod db;
mod fusion;
mod poller;
mod rpc;
mod types;

use crate::config::{get_sqlite_path, get_ttl_secs, load_networks};
use crate::db::DatabaseManager;
use crate::poller::ChainPoller;
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
    let sqlite_path = get_sqlite_path();
    let ttl_secs = get_ttl_secs();
    let networks = load_networks();

    info!("SQLite data directory: {}", sqlite_path);
    info!("TTL: {} seconds ({} minutes)", ttl_secs, ttl_secs / 60);
    info!("Networks: {} chains configured", networks.len());

    // Get chain IDs from networks
    let chain_ids: Vec<u32> = networks.iter().map(|n| n.chain_id).collect();
    info!("Chain IDs: {:?}", chain_ids);

    // Open DatabaseManager (creates per-chain SQLite databases + shared database)
    let db_manager = match DatabaseManager::open(&sqlite_path, &chain_ids) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to open databases: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "Database initialized: {} chain databases + 1 shared database",
        chain_ids.len()
    );

    // Spawn cleanup task
    let db_cleanup = Arc::clone(&db_manager);
    let cleanup_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await;

            // Clean up old data from all databases
            match db_cleanup.cleanup_all(ttl_secs) {
                Ok(stats) => {
                    let total_deleted = stats.transfers_deleted
                        + stats.fusion_plus_deleted
                        + stats.fusion_deleted
                        + stats.crypto2fiat_deleted;
                    if total_deleted > 0 {
                        info!(
                            "Cleanup: removed {} transfers, {} Fusion+ swaps, {} Fusion swaps, {} Crypto2Fiat events",
                            stats.transfers_deleted,
                            stats.fusion_plus_deleted,
                            stats.fusion_deleted,
                            stats.crypto2fiat_deleted
                        );
                    }
                }
                Err(e) => {
                    warn!("Cleanup error: {}", e);
                }
            }

            // Force WAL checkpoint on all databases to release memory
            if let Err(e) = db_cleanup.checkpoint_all() {
                warn!("WAL checkpoint error: {}", e);
            }

            // Log stats every cleanup cycle
            let transfer_count = db_cleanup.get_total_transfer_count().unwrap_or(0);
            let fusion_plus_count = db_cleanup.shared().get_fusion_plus_count().unwrap_or(0);
            let fusion_count = db_cleanup.shared().get_fusion_swap_count().unwrap_or(0);
            let crypto2fiat_count = db_cleanup.shared().get_crypto2fiat_count().unwrap_or(0);
            info!(
                "Database stats: {} transfers, {} Fusion+ swaps, {} Fusion swaps, {} Crypto2Fiat events",
                transfer_count, fusion_plus_count, fusion_count, crypto2fiat_count
            );
        }
    });

    // Spawn poller for each chain
    let mut poller_handles = Vec::new();

    for network in networks {
        // Get chain-specific database
        let chain_db = match db_manager.chain(network.chain_id) {
            Some(db) => db,
            None => {
                error!("No database for chain {}", network.chain_id);
                continue;
            }
        };

        // Get shared database for cross-chain data
        let shared_db = db_manager.shared();
        let chain_name = network.name.to_string();

        let handle = tokio::spawn(async move {
            let mut poller = ChainPoller::new(network, chain_db, shared_db);
            poller.run().await;
        });

        info!("Spawned poller for {}", chain_name);
        poller_handles.push(handle);
    }

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

    // Abort all poller tasks
    for handle in poller_handles {
        handle.abort();
    }
    cleanup_handle.abort();

    info!("Shutdown complete");
}
