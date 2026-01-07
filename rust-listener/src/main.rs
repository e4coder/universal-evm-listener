mod config;
mod db;
mod poller;
mod rpc;
mod types;

use crate::config::{get_sqlite_path, get_ttl_secs, load_networks};
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
    let sqlite_path = get_sqlite_path();
    let ttl_secs = get_ttl_secs();
    let networks = load_networks();

    info!("SQLite path: {}", sqlite_path);
    info!("TTL: {} seconds ({} minutes)", ttl_secs, ttl_secs / 60);
    info!("Networks: {} chains configured", networks.len());

    // Open SQLite database
    let db = match Database::open(&sqlite_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    info!("Database initialized");

    // Spawn cleanup task
    let db_cleanup = Arc::clone(&db);
    let cleanup_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await;

            match db_cleanup.cleanup_old(ttl_secs) {
                Ok(deleted) => {
                    if deleted > 0 {
                        info!("Cleanup: removed {} old transfers", deleted);
                    }
                }
                Err(e) => {
                    warn!("Cleanup error: {}", e);
                }
            }

            // Log stats every cleanup cycle
            match db_cleanup.get_transfer_count() {
                Ok(count) => {
                    info!("Database stats: {} transfers stored", count);
                }
                Err(e) => {
                    warn!("Failed to get stats: {}", e);
                }
            }
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
