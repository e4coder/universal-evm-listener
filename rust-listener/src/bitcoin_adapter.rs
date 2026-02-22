use crate::adapter::ChainAdapter;
use crate::bitcoin_rpc::{BitcoinRpcClient, BitcoinTransaction};
use crate::types::{DecodedBlockRange, MempoolUpdate, Transfer};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

/// Bitcoin adapter implementing ChainAdapter for confirmed blocks + mempool monitoring.
pub struct BitcoinAdapter {
    rpc: Arc<BitcoinRpcClient>,
    chain_id: u32,
    chain_name: &'static str,
}

impl BitcoinAdapter {
    pub fn new(rpc: Arc<BitcoinRpcClient>, chain_id: u32, chain_name: &'static str) -> Self {
        Self {
            rpc,
            chain_id,
            chain_name,
        }
    }
}

#[async_trait]
impl ChainAdapter for BitcoinAdapter {
    async fn get_block_number(&self) -> Result<u64, String> {
        self.rpc.get_block_count().await
    }

    /// Fetch confirmed Bitcoin blocks and decode transactions into Transfer rows.
    /// Bitcoin blocks are large, so we process one at a time (blocks_per_request=1).
    async fn fetch_decoded(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<DecodedBlockRange, String> {
        let mut all_transfers = Vec::new();

        for height in from_block..=to_block {
            let hash = self.rpc.get_block_hash(height).await?;
            let block = self.rpc.get_block_verbose(&hash).await?;

            for tx in &block.tx {
                let transfers =
                    decode_bitcoin_tx(tx, self.chain_id, "confirmed", height, block.time);
                all_transfers.extend(transfers);
            }

            debug!(
                "[{}] Block {} decoded: {} transfers",
                self.chain_name,
                height,
                all_transfers.len()
            );
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
        true
    }

    /// Poll the Bitcoin mempool for new/removed transactions.
    /// Computes delta against the known txid set from the previous cycle.
    async fn poll_mempool(
        &self,
        known_txids: &HashSet<String>,
    ) -> Result<MempoolUpdate, String> {
        // 1. Get current mempool txids
        let current_mempool = self.rpc.get_raw_mempool().await?;
        let current_set: HashSet<String> = current_mempool.into_iter().collect();

        // 2. New txids = in current but not in known
        let new_txids: Vec<String> = current_set.difference(known_txids).cloned().collect();

        // 3. Removed txids = in known but not in current
        let removed_txids: Vec<String> = known_txids.difference(&current_set).cloned().collect();

        debug!(
            "[{}] Mempool poll: {} current, {} new, {} removed",
            self.chain_name,
            current_set.len(),
            new_txids.len(),
            removed_txids.len()
        );

        // 4. Fetch new transactions and decode into pending transfers
        let mut new_transfers = Vec::new();
        if !new_txids.is_empty() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let tx_results = self.rpc.batch_get_transactions(&new_txids).await?;
            for maybe_tx in tx_results {
                if let Some(tx) = maybe_tx {
                    let transfers =
                        decode_bitcoin_tx(&tx, self.chain_id, "pending", 0, now);
                    new_transfers.extend(transfers);
                }
            }
        }

        // 5. Check removed txids: confirmed or dropped?
        let mut confirmed = Vec::new();
        let mut dropped = Vec::new();

        if !removed_txids.is_empty() {
            let tx_results = self.rpc.batch_get_transactions(&removed_txids).await?;
            for (i, maybe_tx) in tx_results.into_iter().enumerate() {
                let txid = &removed_txids[i];
                match maybe_tx {
                    Some(tx) if tx.confirmations.unwrap_or(0) > 0 => {
                        // Transaction was confirmed in a block
                        let block_time = tx.block_time.unwrap_or(0);
                        // We don't have the block height directly from getrawtransaction,
                        // so the confirmed block will be picked up by fetch_decoded.
                        // Mark as confirmed with block_time (block_number=0, will be updated
                        // by the confirmed block processing path).
                        confirmed.push((txid.clone(), 0u64, block_time));
                    }
                    None => {
                        // Transaction not found — truly dropped from mempool
                        dropped.push(txid.clone());
                    }
                    Some(_) => {
                        // Still unconfirmed but not in mempool — likely just evicted
                        // or replaced (RBF). Mark as dropped.
                        dropped.push(txid.clone());
                    }
                }
            }

            if !confirmed.is_empty() || !dropped.is_empty() {
                debug!(
                    "[{}] Mempool removals: {} confirmed, {} dropped",
                    self.chain_name,
                    confirmed.len(),
                    dropped.len()
                );
            }
        }

        Ok(MempoolUpdate {
            new_transfers,
            confirmed,
            dropped,
            current_txids: current_set,
        })
    }
}

/// Decode a Bitcoin transaction into Transfer rows.
/// Each output with a standard address becomes one Transfer.
fn decode_bitcoin_tx(
    tx: &BitcoinTransaction,
    chain_id: u32,
    status: &str,
    block_number: u64,
    block_timestamp: u64,
) -> Vec<Transfer> {
    // Determine sender address from first input
    let from_addr = if tx.vin.first().and_then(|v| v.coinbase.as_ref()).is_some() {
        "coinbase".to_string()
    } else {
        tx.vin
            .first()
            .and_then(|inp| inp.prevout.as_ref())
            .and_then(|p| p.script_pub_key.as_ref())
            .and_then(|spk| spk.address.clone())
            .unwrap_or_else(|| "unknown".to_string())
    };

    tx.vout
        .iter()
        .filter_map(|out| {
            // Skip outputs without a standard address (OP_RETURN, etc.)
            let addr = out.script_pub_key.address.as_ref()?;

            // Convert BTC to satoshis (avoid floating point issues)
            let satoshis = (out.value * 100_000_000.0).round() as u64;

            Some(Transfer {
                chain_id,
                tx_hash: tx.txid.clone(),
                log_index: out.n,
                token: "btc".to_string(),
                from_addr: from_addr.clone(),
                to_addr: addr.clone(),
                value: satoshis.to_string(),
                block_number,
                block_timestamp,
                swap_type: None,
                status: Some(status.to_string()),
            })
        })
        .collect()
}
