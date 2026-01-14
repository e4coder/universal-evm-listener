use serde::{Deserialize, Serialize};

/// ERC20 Transfer event topic (keccak256 of "Transfer(address,address,uint256)")
pub const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

// ============================================================================
// 1inch Fusion+ Constants
// ============================================================================

/// 1inch Fusion+ EscrowFactory contract address (same on all supported chains)
pub const ESCROW_FACTORY: &str = "0xa7bcb4eac8964306f9e3764f67db6a7af6ddf99a";

/// SrcEscrowCreated event topic - emitted on source chain when swap initiated
pub const SRC_ESCROW_CREATED_TOPIC: &str = "0x0e534c62f0afd2fa0f0fa71198e8aa2d549f24daf2bb47de0d5486c7ce9288ca";

/// DstEscrowCreated event topic - emitted on destination chain when resolver creates escrow
pub const DST_ESCROW_CREATED_TOPIC: &str = "0x4d81cba2e6bb297be9304a3fd015ef78782b99f914a881ee9bd2f93291ee6eab";

/// EscrowWithdrawal event topic - emitted when escrow is withdrawn (reveals secret)
pub const ESCROW_WITHDRAWAL_TOPIC: &str = "0xbd74e509ab3bcbbaa9ee979d61e331c3f713f39325be2929dca5e6625e34f5d0";

/// EscrowCancelled event topic - emitted when escrow is cancelled
pub const ESCROW_CANCELLED_TOPIC: &str = "0x7be8ac5ba29ab8ec09d5d66e9fc5d4050be86891af6a8ae794f74b9a4956b7cd";

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

// ============================================================================
// 1inch Fusion+ Data Structures
// ============================================================================

/// Data decoded from SrcEscrowCreated event
#[derive(Debug, Clone)]
pub struct SrcEscrowCreatedData {
    pub order_hash: String,
    pub hashlock: String,
    pub src_maker: String,
    pub src_taker: String,
    pub src_token: String,
    pub src_amount: String,
    pub src_safety_deposit: String,
    pub src_timelocks: String,
    pub dst_maker: String,
    pub dst_amount: String,
    pub dst_token: String,
    pub dst_safety_deposit: String,
    pub dst_chain_id: u32,
}

/// Data decoded from DstEscrowCreated event
#[derive(Debug, Clone)]
pub struct DstEscrowCreatedData {
    pub order_hash: String,
    pub hashlock: String,
    pub dst_maker: String,
    pub dst_taker: String,
    pub dst_token: String,
    pub dst_amount: String,
    pub dst_safety_deposit: String,
    pub dst_timelocks: String,
}

/// Fusion+ swap record stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusionPlusSwap {
    pub order_hash: String,
    pub hashlock: String,
    pub secret: Option<String>,

    // Source chain data
    pub src_chain_id: u32,
    pub src_tx_hash: String,
    pub src_block_number: u64,
    pub src_block_timestamp: u64,
    pub src_log_index: u32,
    pub src_escrow_address: Option<String>,
    pub src_maker: String,
    pub src_taker: String,
    pub src_token: String,
    pub src_amount: String,
    pub src_safety_deposit: String,
    pub src_timelocks: String,
    pub src_status: String,

    // Destination chain data (partially nullable until DstEscrowCreated)
    pub dst_chain_id: u32,
    pub dst_tx_hash: Option<String>,
    pub dst_block_number: Option<u64>,
    pub dst_block_timestamp: Option<u64>,
    pub dst_log_index: Option<u32>,
    pub dst_escrow_address: Option<String>,
    pub dst_maker: String,
    pub dst_taker: Option<String>,
    pub dst_token: String,
    pub dst_amount: String,
    pub dst_safety_deposit: String,
    pub dst_timelocks: Option<String>,
    pub dst_status: String,
}

impl FusionPlusSwap {
    /// Create a new FusionPlusSwap from SrcEscrowCreated event data
    pub fn from_src_created(
        data: &SrcEscrowCreatedData,
        chain_id: u32,
        tx_hash: &str,
        block_number: u64,
        block_timestamp: u64,
        log_index: u32,
    ) -> Self {
        Self {
            order_hash: data.order_hash.clone(),
            hashlock: data.hashlock.clone(),
            secret: None,

            src_chain_id: chain_id,
            src_tx_hash: tx_hash.to_string(),
            src_block_number: block_number,
            src_block_timestamp: block_timestamp,
            src_log_index: log_index,
            src_escrow_address: None,
            src_maker: data.src_maker.clone(),
            src_taker: data.src_taker.clone(),
            src_token: data.src_token.clone(),
            src_amount: data.src_amount.clone(),
            src_safety_deposit: data.src_safety_deposit.clone(),
            src_timelocks: data.src_timelocks.clone(),
            src_status: "created".to_string(),

            dst_chain_id: data.dst_chain_id,
            dst_tx_hash: None,
            dst_block_number: None,
            dst_block_timestamp: None,
            dst_log_index: None,
            dst_escrow_address: None,
            dst_maker: data.dst_maker.clone(),
            dst_taker: None,
            dst_token: data.dst_token.clone(),
            dst_amount: data.dst_amount.clone(),
            dst_safety_deposit: data.dst_safety_deposit.clone(),
            dst_timelocks: None,
            dst_status: "pending".to_string(),
        }
    }
}
