use crate::types::{ChainType, NetworkConfig};
use std::env;

/// Get Alchemy RPC URL for a network
fn alchemy_url(network: &str, api_key: &str) -> String {
    format!("https://{}.g.alchemy.com/v2/{}", network, api_key)
}

/// Helper to create an EVM NetworkConfig with common defaults
fn evm_network(
    chain_id: u32,
    name: &'static str,
    rpc_url: String,
    blocks_per_request: u64,
    concurrent_fetches: usize,
    poll_interval_ms: u64,
    confirmation_blocks: u64,
) -> NetworkConfig {
    NetworkConfig {
        chain_id,
        name,
        rpc_url,
        chain_type: ChainType::Evm,
        blocks_per_request,
        concurrent_fetches,
        poll_interval_ms,
        confirmation_blocks,
        rpc_user: None,
        rpc_password: None,
    }
}

/// Load all supported networks with Alchemy RPC URLs
pub fn load_networks() -> Vec<NetworkConfig> {
    let api_key = env::var("ALCHEMY_API_KEY").expect("ALCHEMY_API_KEY must be set");

    let mut networks = vec![
        // Dense tier — 10 blocks × 10 concurrent = 100 blocks/batch
        evm_network(1, "Ethereum", alchemy_url("eth-mainnet", &api_key), 10, 10, 500, 12),
        // Base: ~2s blocks, very dense (~400 transfers/block), keep block range small
        evm_network(8453, "Base", alchemy_url("base-mainnet", &api_key), 10, 15, 300, 1),
        // Dense tier — 50 blocks × 15 concurrent = 750 blocks/batch
        // Arbitrum ~250ms block time (~240 blocks/min) needs fast polling
        evm_network(42161, "Arbitrum One", alchemy_url("arb-mainnet", &api_key), 50, 15, 100, 1),
        // Polygon: ~2s blocks, extremely dense (~600 transfers/block), keep block range small
        evm_network(137, "Polygon", alchemy_url("polygon-mainnet", &api_key), 10, 15, 300, 50),
        // BNB: ~3s blocks, moderate density (~120 transfers/block)
        evm_network(56, "BNB Smart Chain", alchemy_url("bnb-mainnet", &api_key), 30, 15, 500, 3),
        // Medium tier — 50 blocks × 5 concurrent = 250 blocks/batch
        evm_network(10, "OP Mainnet", alchemy_url("opt-mainnet", &api_key), 50, 5, 500, 1),
        evm_network(43114, "Avalanche", alchemy_url("avax-mainnet", &api_key), 50, 5, 500, 1),
        evm_network(100, "Gnosis", alchemy_url("gnosis-mainnet", &api_key), 50, 5, 500, 12),
        evm_network(1868, "Soneium Mainnet", alchemy_url("soneium-mainnet", &api_key), 50, 5, 500, 1),
        // Sparse tier — 200 blocks × 3 concurrent = 600 blocks/batch
        evm_network(59144, "Linea Mainnet", alchemy_url("linea-mainnet", &api_key), 200, 3, 1000, 1),
        evm_network(130, "Unichain", alchemy_url("unichain-mainnet", &api_key), 200, 3, 1000, 1),
        evm_network(146, "Sonic", alchemy_url("sonic-mainnet", &api_key), 200, 3, 1000, 1),
        evm_network(57073, "Ink", alchemy_url("ink-mainnet", &api_key), 200, 3, 1000, 1),
    ];

    // Conditionally add Bitcoin if BITCOIN_RPC_URL is set
    if let Ok(btc_url) = env::var("BITCOIN_RPC_URL") {
        networks.push(NetworkConfig {
            chain_id: 0,
            name: "Bitcoin",
            rpc_url: btc_url,
            chain_type: ChainType::Bitcoin,
            blocks_per_request: 1,
            concurrent_fetches: 2,
            poll_interval_ms: 5000,
            confirmation_blocks: 6,
            rpc_user: env::var("BITCOIN_RPC_USER").ok(),
            rpc_password: env::var("BITCOIN_RPC_PASSWORD").ok(),
        });
    }

    networks
}

/// Get PostgreSQL database URL from environment
pub fn get_database_url() -> String {
    env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

/// Get TTL in seconds from environment
pub fn get_ttl_secs() -> u64 {
    env::var("TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600) // Default 1 hour
}
