use crate::adapter::ChainAdapter;
use crate::fusion::{
    compute_hashlock_from_secret, decode_crypto2fiat_event, decode_dst_escrow_created,
    decode_escrow_withdrawal, decode_order_filled, decode_src_escrow_created,
};
use crate::rpc::RpcClient;
use crate::types::{
    Crypto2FiatEvent, DecodedBlockRange, FpAction, FusionAction, FusionPlusSwap, FusionSwap,
    Log, Transfer, AGGREGATION_ROUTER_V6, AGGREGATION_ROUTER_ZKSYNC, CRYPTO2FIAT_TOPIC,
    DST_ESCROW_CREATED_TOPIC, ESCROW_CANCELLED_TOPIC, ESCROW_FACTORY,
    ESCROW_WITHDRAWAL_TOPIC, ORDER_CANCELLED_TOPIC, ORDER_FILLED_TOPIC,
    SRC_ESCROW_CREATED_TOPIC, TRANSFER_TOPIC,
};
use async_trait::async_trait;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

/// EVM-specific implementation of ChainAdapter.
/// Handles eth_getLogs, ABI decoding, 0x-prefixed addresses, topic hashing.
pub struct EvmAdapter {
    rpc: Arc<RpcClient>,
    chain_id: u32,
    chain_name: &'static str,
    timestamp_cache: Mutex<HashMap<u64, u64>>,
}

impl EvmAdapter {
    pub fn new(rpc: Arc<RpcClient>, chain_id: u32, chain_name: &'static str) -> Self {
        Self {
            rpc,
            chain_id,
            chain_name,
            timestamp_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Fetch all logs for a block range using a single JSON-RPC batch request.
    /// All 5 eth_getLogs calls are batched into one HTTP POST.
    async fn fetch_all_logs(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<FetchedLogs, String> {
        debug!(
            "[{}] Fetching all logs for blocks {} to {}",
            self.chain_name, from_block, to_block
        );

        let from_hex = format!("0x{:x}", from_block);
        let to_hex = format!("0x{:x}", to_block);

        let router_address = if self.chain_id == 324 {
            AGGREGATION_ROUTER_ZKSYNC
        } else {
            AGGREGATION_ROUTER_V6
        };

        let filters = vec![
            // [0] Fusion+ factory events (SrcEscrowCreated + DstEscrowCreated)
            json!({
                "fromBlock": &from_hex, "toBlock": &to_hex,
                "address": ESCROW_FACTORY,
                "topics": [[SRC_ESCROW_CREATED_TOPIC, DST_ESCROW_CREATED_TOPIC]]
            }),
            // [1] Fusion+ escrow events (Withdrawal + Cancelled) from any address
            json!({
                "fromBlock": &from_hex, "toBlock": &to_hex,
                "topics": [[ESCROW_WITHDRAWAL_TOPIC, ESCROW_CANCELLED_TOPIC]]
            }),
            // [2] Fusion single-chain events (OrderFilled + OrderCancelled)
            json!({
                "fromBlock": &from_hex, "toBlock": &to_hex,
                "address": router_address,
                "topics": [[ORDER_FILLED_TOPIC, ORDER_CANCELLED_TOPIC]]
            }),
            // [3] Crypto2Fiat events from any address
            json!({
                "fromBlock": &from_hex, "toBlock": &to_hex,
                "topics": [CRYPTO2FIAT_TOPIC]
            }),
            // [4] ERC20 Transfer events (the main data)
            json!({
                "fromBlock": &from_hex, "toBlock": &to_hex,
                "topics": [TRANSFER_TOPIC]
            }),
        ];

        let mut results = self
            .rpc
            .batch_get_logs(&filters)
            .await
            .map_err(|e| format!("Batch getLogs failed: {}", e))?;

        // Destructure the 5 results by consuming in reverse order (avoids clone)
        let transfer_result = results.pop().unwrap();
        let c2f_result = results.pop().unwrap();
        let fusion_result = results.pop().unwrap();
        let fp_escrow_result = results.pop().unwrap();
        let fp_factory_result = results.pop().unwrap();

        // Same error policy: fusion/c2f default to empty on error, transfers propagate
        Ok(FetchedLogs {
            fusion_plus_factory_logs: fp_factory_result.unwrap_or_default(),
            fusion_plus_escrow_logs: fp_escrow_result.unwrap_or_default(),
            fusion_logs: fusion_result.unwrap_or_default(),
            crypto2fiat_logs: c2f_result.unwrap_or_default(),
            transfer_logs: transfer_result
                .map_err(|e| format!("Failed to get transfer logs: {}", e))?,
        })
    }

    /// Resolve timestamps for all block numbers found in logs.
    /// Uses internal cache and batch-fetches missing timestamps via RPC.
    async fn resolve_timestamps(&self, all_logs: &[&[Log]]) -> Result<HashMap<u64, u64>, String> {
        let mut needed: HashSet<u64> = HashSet::new();
        for logs in all_logs {
            for log in *logs {
                needed.insert(log.block_number_u64());
            }
        }

        if needed.is_empty() {
            return Ok(HashMap::new());
        }

        let mut timestamps: HashMap<u64, u64> = HashMap::with_capacity(needed.len());
        let mut uncached: Vec<u64> = Vec::new();

        // Check cache for existing timestamps
        {
            let cache = self.timestamp_cache.lock().unwrap();
            for &block_num in &needed {
                if let Some(&ts) = cache.get(&block_num) {
                    timestamps.insert(block_num, ts);
                } else {
                    uncached.push(block_num);
                }
            }
        }

        // Batch-fetch missing timestamps
        if !uncached.is_empty() {
            let results = self
                .rpc
                .batch_get_blocks(&uncached)
                .await
                .map_err(|e| format!("Batch getBlockByNumber failed: {}", e))?;
            for (block_num, result) in uncached.iter().zip(results.into_iter()) {
                match result {
                    Ok(block) => {
                        timestamps.insert(*block_num, block.timestamp_u64());
                    }
                    Err(e) => return Err(format!("Failed to get block {}: {}", block_num, e)),
                }
            }

            // Update cache with newly fetched timestamps and prune old entries
            let mut cache = self.timestamp_cache.lock().unwrap();
            for (&block_num, &ts) in &timestamps {
                cache.insert(block_num, ts);
            }
            if let Some(&max_block) = needed.iter().max() {
                let cutoff = max_block.saturating_sub(500);
                cache.retain(|&block, _| block >= cutoff);
            }
        }

        Ok(timestamps)
    }

    /// Decode transfer logs into Transfer structs with swap_type labels.
    fn decode_transfers(
        &self,
        logs: &[Log],
        timestamps: &HashMap<u64, u64>,
        swap_type_map: &HashMap<String, &'static str>,
    ) -> Result<Vec<Transfer>, String> {
        let mut transfers = Vec::with_capacity(logs.len());
        for log in logs {
            if log.topics.len() < 3 {
                continue;
            }
            let block_number = log.block_number_u64();
            let timestamp = *timestamps
                .get(&block_number)
                .ok_or_else(|| format!("Missing timestamp for block {}", block_number))?;
            let swap_type = swap_type_map
                .get(&log.transaction_hash.to_lowercase())
                .map(|s| s.to_string());
            transfers.push(Transfer {
                chain_id: self.chain_id,
                tx_hash: log.transaction_hash.clone(),
                log_index: log.log_index_u32(),
                token: log.address.to_lowercase(),
                from_addr: format!("0x{}", &log.topics[1][26..]),
                to_addr: format!("0x{}", &log.topics[2][26..]),
                value: log.data.clone(),
                block_number,
                block_timestamp: timestamp,
                swap_type,
                status: None,
            });
        }
        Ok(transfers)
    }

    /// Decode Fusion+ factory and escrow logs into FpAction enums.
    fn decode_fp_events(
        &self,
        factory_logs: &[Log],
        escrow_logs: &[Log],
        timestamps: &HashMap<u64, u64>,
    ) -> Result<Vec<FpAction>, String> {
        let mut actions = Vec::new();

        for log in factory_logs {
            if log.topics.is_empty() {
                continue;
            }
            let ts = *timestamps
                .get(&log.block_number_u64())
                .ok_or_else(|| format!("Missing timestamp for block {}", log.block_number_u64()))?;

            if log.topics[0].to_lowercase() == SRC_ESCROW_CREATED_TOPIC {
                if let Some(data) = decode_src_escrow_created(&log.data) {
                    let swap = FusionPlusSwap::from_src_created(
                        &data,
                        self.chain_id,
                        &log.transaction_hash,
                        log.block_number_u64(),
                        ts,
                        log.log_index_u32(),
                    );
                    info!(
                        "[{}] Fusion+ SrcEscrow created: order_hash={} dst_chain={}",
                        self.chain_name, data.order_hash, data.dst_chain_id
                    );
                    actions.push(FpAction::SrcEscrowCreated { swap });
                } else {
                    warn!(
                        "[{}] Failed to decode SrcEscrowCreated data",
                        self.chain_name
                    );
                }
            } else if log.topics[0].to_lowercase() == DST_ESCROW_CREATED_TOPIC {
                if let Some(data) = decode_dst_escrow_created(&log.data) {
                    info!(
                        "[{}] Fusion+ DstEscrow created: order_hash={}",
                        self.chain_name, data.order_hash
                    );
                    actions.push(FpAction::DstEscrowCreated {
                        order_hash: data.order_hash.clone(),
                        data,
                        chain_id: self.chain_id,
                        tx_hash: log.transaction_hash.clone(),
                        block_number: log.block_number_u64(),
                        block_timestamp: ts,
                        log_index: log.log_index_u32(),
                        escrow_address: log.address.clone(),
                    });
                } else {
                    warn!(
                        "[{}] Failed to decode DstEscrowCreated data",
                        self.chain_name
                    );
                }
            }
        }

        for log in escrow_logs {
            if log.topics.is_empty() {
                continue;
            }
            let ts = *timestamps
                .get(&log.block_number_u64())
                .ok_or_else(|| format!("Missing timestamp for block {}", log.block_number_u64()))?;

            if log.topics[0].to_lowercase() == ESCROW_WITHDRAWAL_TOPIC {
                if let Some(secret) = decode_escrow_withdrawal(&log.data) {
                    if let Some(hashlock) = compute_hashlock_from_secret(&secret) {
                        debug!(
                            "[{}] Fusion+ withdrawal from escrow {} with hashlock {}",
                            self.chain_name, log.address, hashlock
                        );
                        actions.push(FpAction::EscrowWithdrawal {
                            secret,
                            hashlock,
                            chain_id: self.chain_id,
                            tx_hash: log.transaction_hash.clone(),
                            block_number: log.block_number_u64(),
                            block_timestamp: ts,
                            log_index: log.log_index_u32(),
                            escrow_address: log.address.clone(),
                        });
                    } else {
                        warn!(
                            "[{}] Failed to compute hashlock from secret",
                            self.chain_name
                        );
                    }
                } else {
                    debug!(
                        "[{}] Failed to decode EscrowWithdrawal data",
                        self.chain_name
                    );
                }
            } else if log.topics[0].to_lowercase() == ESCROW_CANCELLED_TOPIC {
                debug!(
                    "[{}] Fusion+ escrow cancelled: {}",
                    self.chain_name, log.address
                );
                actions.push(FpAction::EscrowCancelled {
                    escrow_address: log.address.clone(),
                });
            }
        }

        Ok(actions)
    }

    /// Decode Fusion single-chain events into FusionAction enums.
    /// Uses in-memory maker/taker lookup from decoded transfers (replaces DB read-back).
    fn decode_fusion_events(
        &self,
        logs: &[Log],
        timestamps: &HashMap<u64, u64>,
        transfers: &[Transfer],
    ) -> Result<Vec<FusionAction>, String> {
        if logs.is_empty() {
            return Ok(Vec::new());
        }

        // Build tx_hash → (first_transfer_idx, last_transfer_idx) from decoded transfers.
        // OrderFilled events and their transfers are always in the same tx → same block → same range.
        let mut tx_bounds: HashMap<String, (usize, usize)> = HashMap::new();
        for (i, t) in transfers.iter().enumerate() {
            tx_bounds
                .entry(t.tx_hash.to_lowercase())
                .and_modify(|(first, last)| {
                    if t.log_index < transfers[*first].log_index {
                        *first = i;
                    }
                    if t.log_index > transfers[*last].log_index {
                        *last = i;
                    }
                })
                .or_insert((i, i));
        }

        let mut actions = Vec::new();

        for log in logs {
            if log.topics.is_empty() {
                continue;
            }
            let ts = *timestamps
                .get(&log.block_number_u64())
                .ok_or_else(|| format!("Missing timestamp for block {}", log.block_number_u64()))?;
            let topic0 = log.topics[0].to_lowercase();

            let status = if topic0 == ORDER_FILLED_TOPIC {
                "filled"
            } else if topic0 == ORDER_CANCELLED_TOPIC {
                "cancelled"
            } else {
                continue;
            };

            let data = match decode_order_filled(&log.topics, &log.data) {
                Some(d) => d,
                None => {
                    debug!(
                        "[{}] Failed to decode Order{} data",
                        self.chain_name, status
                    );
                    continue;
                }
            };

            let remaining_hex = data.remaining.trim_start_matches("0x");
            let is_partial = !remaining_hex.chars().all(|c| c == '0');

            // In-memory maker/taker lookup (replaces Database::get_first_last_transfers_on)
            let tx_hash_lower = log.transaction_hash.to_lowercase();
            let (maker, taker, maker_token, taker_token, maker_amount, taker_amount) =
                if let Some(&(first_idx, last_idx)) = tx_bounds.get(&tx_hash_lower) {
                    let first = &transfers[first_idx];
                    let last = &transfers[last_idx];
                    (
                        first.from_addr.clone(),
                        Some(last.to_addr.clone()),
                        Some(first.token.clone()),
                        Some(last.token.clone()),
                        Some(first.value.clone()),
                        Some(last.value.clone()),
                    )
                } else {
                    (String::new(), None, None, None, None, None)
                };

            let swap = FusionSwap {
                order_hash: data.order_hash.clone(),
                chain_id: self.chain_id,
                tx_hash: log.transaction_hash.clone(),
                block_number: log.block_number_u64(),
                block_timestamp: ts,
                log_index: log.log_index_u32(),
                maker,
                taker,
                maker_token,
                taker_token,
                maker_amount,
                taker_amount,
                remaining: data.remaining.clone(),
                is_partial_fill: is_partial,
                status: status.to_string(),
            };

            info!(
                "[{}] Fusion {} order: order_hash={} maker={} taker={:?} tx={}",
                self.chain_name, status, data.order_hash, swap.maker, swap.taker, log.transaction_hash
            );

            if status == "filled" {
                actions.push(FusionAction::OrderFilled { swap });
            } else {
                actions.push(FusionAction::OrderCancelled { swap });
            }
        }

        Ok(actions)
    }

    /// Decode Crypto2Fiat event logs.
    fn decode_c2f_events(
        &self,
        logs: &[Log],
        timestamps: &HashMap<u64, u64>,
    ) -> Result<Vec<Crypto2FiatEvent>, String> {
        let mut events = Vec::new();
        for log in logs {
            if log.topics.is_empty() {
                continue;
            }
            let ts = *timestamps
                .get(&log.block_number_u64())
                .ok_or_else(|| format!("Missing timestamp for block {}", log.block_number_u64()))?;

            if let Some(mut event) = decode_crypto2fiat_event(log) {
                event.chain_id = self.chain_id;
                event.tx_hash = log.transaction_hash.clone();
                event.block_number = log.block_number_u64();
                event.block_timestamp = ts;
                event.log_index = log.log_index_u32();
                info!(
                    "[{}] Crypto2Fiat: order_id={} token={} amount={} recipient={} tx={}",
                    self.chain_name, event.order_id, event.token, event.amount,
                    event.recipient, event.tx_hash
                );
                events.push(event);
            } else {
                debug!(
                    "[{}] Failed to decode Crypto2Fiat event",
                    self.chain_name
                );
            }
        }
        Ok(events)
    }
}

/// Internal struct for raw EVM logs before decoding (not exposed outside adapter).
struct FetchedLogs {
    fusion_plus_factory_logs: Vec<Log>,
    fusion_plus_escrow_logs: Vec<Log>,
    fusion_logs: Vec<Log>,
    crypto2fiat_logs: Vec<Log>,
    transfer_logs: Vec<Log>,
}

#[async_trait]
impl ChainAdapter for EvmAdapter {
    async fn get_block_number(&self) -> Result<u64, String> {
        self.rpc
            .get_block_number()
            .await
            .map_err(|e| format!("{}", e))
    }

    async fn fetch_decoded(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<DecodedBlockRange, String> {
        // 1. Fetch all raw logs via batch RPC
        let logs = self.fetch_all_logs(from_block, to_block).await?;

        // 2. Resolve timestamps for all blocks in the logs
        let timestamps = self
            .resolve_timestamps(&[
                &logs.fusion_plus_factory_logs,
                &logs.fusion_plus_escrow_logs,
                &logs.fusion_logs,
                &logs.crypto2fiat_logs,
                &logs.transfer_logs,
            ])
            .await?;

        // 3. Build swap_type map for labeling transfers
        let mut swap_type_map: HashMap<String, &'static str> = HashMap::new();
        for log in &logs.fusion_plus_factory_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion_plus");
        }
        for log in &logs.fusion_plus_escrow_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion_plus");
        }
        for log in &logs.fusion_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "fusion");
        }
        for log in &logs.crypto2fiat_logs {
            swap_type_map.insert(log.transaction_hash.to_lowercase(), "crypto_to_fiat");
        }

        if !logs.transfer_logs.is_empty() {
            info!(
                "[{}] Found {} Transfer events in blocks {}-{}",
                self.chain_name,
                logs.transfer_logs.len(),
                from_block,
                to_block
            );
        }

        // 4. Decode transfers
        let transfers =
            self.decode_transfers(&logs.transfer_logs, &timestamps, &swap_type_map)?;

        // 5. Decode Fusion+ events
        let fusion_plus_actions = self.decode_fp_events(
            &logs.fusion_plus_factory_logs,
            &logs.fusion_plus_escrow_logs,
            &timestamps,
        )?;

        // 6. Decode Fusion single-chain events (uses in-memory transfer lookup)
        let fusion_actions =
            self.decode_fusion_events(&logs.fusion_logs, &timestamps, &transfers)?;

        // 7. Decode Crypto2Fiat events
        let crypto2fiat_events =
            self.decode_c2f_events(&logs.crypto2fiat_logs, &timestamps)?;

        Ok(DecodedBlockRange {
            from_block,
            to_block,
            transfers,
            fusion_plus_actions,
            fusion_actions,
            crypto2fiat_events,
        })
    }
}
