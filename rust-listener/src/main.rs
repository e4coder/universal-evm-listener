#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod config;
mod db;
mod fusion;
mod poller;
mod rpc;
mod types;

use crate::config::{get_database_url, get_ttl_secs, load_networks};
use crate::db::Database;
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
    let db = match Database::new(&database_url).await {
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

    // Spawn poller for each chain
    let mut poller_handles = Vec::new();

    for network in networks {
        let db_clone = Arc::clone(&db);
        let chain_name = network.name.to_string();

        let handle = tokio::spawn(async move {
            let mut poller = ChainPoller::new(network, db_clone);
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
