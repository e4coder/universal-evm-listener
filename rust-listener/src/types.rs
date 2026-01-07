use serde::{Deserialize, Serialize};

/// ERC20 Transfer event topic (keccak256 of "Transfer(address,address,uint256)")
pub const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Network configuration for a blockchain
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub chain_id: u32,
    pub name: &'static str,
    pub rpc_url: String,
}

/// Transfer event data to store in SQLite
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub chain_id: u32,
    pub tx_hash: String,
    pub log_index: u32,
    pub token: String,
    pub from_addr: String,
    pub to_addr: String,
    pub value: String,
    pub block_number: u64,
    pub block_timestamp: u64,
}

/// JSON-RPC response structures
#[derive(Debug, Deserialize)]
pub struct RpcResponse<T> {
    pub result: Option<T>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Log entry from eth_getLogs
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    pub block_number: String,
    pub transaction_hash: String,
    pub log_index: String,
}

impl Log {
    /// Parse block number from hex string
    pub fn block_number_u64(&self) -> u64 {
        u64::from_str_radix(self.block_number.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Parse log index from hex string
    pub fn log_index_u32(&self) -> u32 {
        u32::from_str_radix(self.log_index.trim_start_matches("0x"), 16).unwrap_or(0)
    }
}

/// Block data from eth_getBlockByNumber
#[derive(Debug, Deserialize)]
pub struct Block {
    pub timestamp: String,
}

impl Block {
    /// Parse timestamp from hex string
    pub fn timestamp_u64(&self) -> u64 {
        u64::from_str_radix(self.timestamp.trim_start_matches("0x"), 16).unwrap_or(0)
    }
}
