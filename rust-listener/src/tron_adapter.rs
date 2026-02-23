use crate::adapter::ChainAdapter;
use crate::tron_rpc::{hex_to_tron_address, TronBlock, TronRpcClient, TronTransactionInfo};
use crate::types::{DecodedBlockRange, Transfer};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::debug;

/// TRC20 Transfer event topic (same as ERC20): keccak256("Transfer(address,address,uint256)")
/// Tron topics are hex without 0x prefix.
const TRANSFER_TOPIC: &str =
    "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

pub struct TronAdapter {
    rpc: Arc<TronRpcClient>,
    chain_id: u32,
    chain_name: &'static str,
}

impl TronAdapter {
    pub fn new(rpc: Arc<TronRpcClient>, chain_id: u32, chain_name: &'static str) -> Self {
        Self {
            rpc,
            chain_id,
            chain_name,
        }
    }
}

#[async_trait]
impl ChainAdapter for TronAdapter {
    async fn get_block_number(&self) -> Result<u64, String> {
        self.rpc.get_now_block().await
    }

    async fn fetch_decoded(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<DecodedBlockRange, String> {
        let mut all_transfers = Vec::new();

        for block_num in from_block..=to_block {
            let block = self.rpc.get_block_by_num(block_num).await?;
            let tx_infos = self
                .rpc
                .get_transaction_info_by_block_num(block_num)
                .await?;

            let block_timestamp = block.block_header.raw_data.timestamp / 1000;

            // 1. Native TRX transfers from TransferContract
            let trx_transfers =
                decode_native_trx_transfers(&block, self.chain_id, block_num, block_timestamp);

            // 2. TRC20 token transfers from event logs
            let trc20_transfers =
                decode_trc20_transfers(&tx_infos, self.chain_id, block_num, block_timestamp);

            debug!(
                "[{}] Block {} decoded: {} TRX + {} TRC20 transfers from {} txs",
                self.chain_name,
                block_num,
                trx_transfers.len(),
                trc20_transfers.len(),
                block.transactions.len()
            );

            all_transfers.extend(trx_transfers);
            all_transfers.extend(trc20_transfers);
        }

        Ok(DecodedBlockRange {
            from_block,
            to_block,
            transfers: all_transfers,
            fusion_plus_actions: vec![],
            fusion_actions: vec![],
            crypto2fiat_events: vec![],
        })
    }

    fn supports_mempool(&self) -> bool {
        false // Tron has no public mempool API
    }
}

/// Parse native TRX transfers from TransferContract in block transactions.
/// log_index starts at 0 for native transfers.
fn decode_native_trx_transfers(
    block: &TronBlock,
    chain_id: u32,
    block_num: u64,
    block_timestamp: u64,
) -> Vec<Transfer> {
    let mut transfers = Vec::new();
    let mut log_index: u32 = 0;

    for tx in &block.transactions {
        // Only process successful transactions
        let success = tx
            .ret
            .first()
            .and_then(|r| r.contract_ret.as_deref())
            .map(|s| s == "SUCCESS")
            .unwrap_or(false);
        if !success {
            continue;
        }

        let contract = match tx.raw_data.contract.first() {
            Some(c) => c,
            None => continue,
        };

        if contract.contract_type != "TransferContract" {
            continue;
        }

        // TransferContract parameter.value has: owner_address, to_address, amount
        let value = &contract.parameter.value;

        let owner_hex = match value.get("owner_address").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let to_hex = match value.get("to_address").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let amount = value
            .get("amount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let from_addr = match hex_to_tron_address(owner_hex) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let to_addr = match hex_to_tron_address(to_hex) {
            Ok(a) => a,
            Err(_) => continue,
        };

        transfers.push(Transfer {
            chain_id,
            tx_hash: tx.tx_id.clone(),
            log_index,
            token: "trx".to_string(),
            from_addr,
            to_addr,
            value: amount.to_string(),
            block_number: block_num,
            block_timestamp,
            swap_type: None,
            status: None,
        });
        log_index += 1;
    }

    transfers
}

/// Parse TRC20 Transfer events from transaction info logs.
/// log_index starts at 10000 to avoid collision with native TRX indices.
fn decode_trc20_transfers(
    tx_infos: &[TronTransactionInfo],
    chain_id: u32,
    block_num: u64,
    block_timestamp: u64,
) -> Vec<Transfer> {
    let mut transfers = Vec::new();
    let mut log_index: u32 = 10000;

    for info in tx_infos {
        let tx_hash = match &info.id {
            Some(id) => id,
            None => continue,
        };

        // Only process successful transactions
        let success = info
            .receipt
            .as_ref()
            .and_then(|r| r.result.as_deref())
            .map(|s| s == "SUCCESS")
            .unwrap_or(true); // no receipt = assume success (simple transfers)
        if !success {
            continue;
        }

        for log in &info.log {
            // Need at least 3 topics for Transfer(from, to, value)
            if log.topics.len() < 3 {
                continue;
            }

            // Check Transfer event topic (hex, no 0x prefix)
            if log.topics[0] != TRANSFER_TOPIC {
                continue;
            }

            // topics[1] = from address (32 bytes, last 20 bytes are the address)
            let from_hex = extract_address_from_topic(&log.topics[1]);
            let to_hex = extract_address_from_topic(&log.topics[2]);

            let from_addr = match hex_to_tron_address(&from_hex) {
                Ok(a) => a,
                Err(_) => continue,
            };
            let to_addr = match hex_to_tron_address(&to_hex) {
                Ok(a) => a,
                Err(_) => continue,
            };

            // data = uint256 value (hex, no 0x prefix)
            let value = log
                .data
                .as_deref()
                .and_then(|d| {
                    let trimmed = d.trim_start_matches('0');
                    if trimmed.is_empty() {
                        Some("0".to_string())
                    } else {
                        u128::from_str_radix(trimmed, 16)
                            .ok()
                            .map(|v| v.to_string())
                    }
                })
                .unwrap_or_else(|| "0".to_string());

            // Token address from log.address (hex 41-prefix → Base58Check)
            let token = match &log.address {
                Some(addr) => match hex_to_tron_address(addr) {
                    Ok(a) => a,
                    Err(_) => continue,
                },
                None => continue,
            };

            transfers.push(Transfer {
                chain_id,
                tx_hash: tx_hash.clone(),
                log_index,
                token,
                from_addr,
                to_addr,
                value,
                block_number: block_num,
                block_timestamp,
                swap_type: None,
                status: None,
            });
            log_index += 1;
        }
    }

    transfers
}

/// Extract a 20-byte address from a 32-byte topic hex string.
/// Topic is 64 hex chars, address is last 40 hex chars, prepend "41" for Tron.
fn extract_address_from_topic(topic: &str) -> String {
    let hex = topic.trim_start_matches("0x");
    if hex.len() >= 40 {
        // Last 40 hex chars = 20-byte address, prepend 41 for Tron mainnet
        format!("41{}", &hex[hex.len() - 40..])
    } else {
        hex.to_string()
    }
}
