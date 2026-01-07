use crate::types::NetworkConfig;
use std::env;

/// Get Alchemy RPC URL for a network
fn alchemy_url(network: &str, api_key: &str) -> String {
    format!("https://{}.g.alchemy.com/v2/{}", network, api_key)
}

/// Load all supported networks with Alchemy RPC URLs
pub fn load_networks() -> Vec<NetworkConfig> {
    let api_key = env::var("ALCHEMY_API_KEY").expect("ALCHEMY_API_KEY must be set");

    vec![
        NetworkConfig {
            chain_id: 1,
            name: "Ethereum",
            rpc_url: alchemy_url("eth-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 42161,
            name: "Arbitrum One",
            rpc_url: alchemy_url("arb-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 137,
            name: "Polygon",
            rpc_url: alchemy_url("polygon-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 10,
            name: "OP Mainnet",
            rpc_url: alchemy_url("opt-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 8453,
            name: "Base",
            rpc_url: alchemy_url("base-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 100,
            name: "Gnosis",
            rpc_url: alchemy_url("gnosis-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 56,
            name: "BNB Smart Chain",
            rpc_url: alchemy_url("bnb-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 43114,
            name: "Avalanche",
            rpc_url: alchemy_url("avax-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 59144,
            name: "Linea Mainnet",
            rpc_url: alchemy_url("linea-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 130,
            name: "Unichain",
            rpc_url: alchemy_url("unichain-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 1868,
            name: "Soneium Mainnet",
            rpc_url: alchemy_url("soneium-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 146,
            name: "Sonic",
            rpc_url: alchemy_url("sonic-mainnet", &api_key),
        },
        NetworkConfig {
            chain_id: 57073,
            name: "Ink",
            rpc_url: alchemy_url("ink-mainnet", &api_key),
        },
    ]
}

/// Get SQLite database path from environment
pub fn get_sqlite_path() -> String {
    env::var("SQLITE_PATH").unwrap_or_else(|_| "data/transfers.db".to_string())
}

/// Get TTL in seconds from environment
pub fn get_ttl_secs() -> u64 {
    env::var("TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600) // Default 10 minutes
}
