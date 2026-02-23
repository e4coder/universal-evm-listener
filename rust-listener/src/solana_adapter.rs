use crate::adapter::ChainAdapter;
use crate::solana_rpc::{SolanaBlock, SolanaRpcClient, SolanaTransactionEntry, SolanaTransactionMeta, TokenBalance};
use crate::types::{DecodedBlockRange, Transfer};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

pub struct SolanaAdapter {
    rpc: Arc<SolanaRpcClient>,
    chain_id: u32,
    chain_name: &'static str,
}

impl SolanaAdapter {
    pub fn new(rpc: Arc<SolanaRpcClient>, chain_id: u32, chain_name: &'static str) -> Self {
        Self {
            rpc,
            chain_id,
            chain_name,
        }
    }
}

#[async_trait]
impl ChainAdapter for SolanaAdapter {
    async fn get_block_number(&self) -> Result<u64, String> {
        self.rpc.get_slot().await
    }

    async fn fetch_decoded(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<DecodedBlockRange, String> {
        let mut all_transfers = Vec::new();

        for slot in from_block..=to_block {
            match self.rpc.get_block(slot).await? {
                None => {
                    // Skipped slot — no block produced. Normal on Solana.
                    continue;
                }
                Some(block) => {
                    let block_time = block.block_time.unwrap_or(0);
                    let transfers = decode_solana_block(&block, self.chain_id, slot, block_time);

                    debug!(
                        "[{}] Slot {} decoded: {} transfers from {} txs",
                        self.chain_name,
                        slot,
                        transfers.len(),
                        block.transactions.len()
                    );

                    all_transfers.extend(transfers);
                }
            }
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
        false // Solana uses Gulf Stream — no mempool
    }
}

/// Decode all transfers from a Solana block using balance diffs.
fn decode_solana_block(
    block: &SolanaBlock,
    chain_id: u32,
    slot: u64,
    block_time: u64,
) -> Vec<Transfer> {
    let mut transfers = Vec::new();

    for tx_entry in &block.transactions {
        let meta = match &tx_entry.meta {
            Some(m) => m,
            None => continue,
        };

        // Skip failed transactions
        if meta.err.is_some() {
            continue;
        }

        let signature = tx_entry
            .transaction
            .signatures
            .first()
            .cloned()
            .unwrap_or_default();

        // Build full account keys list (static + loaded for versioned txs)
        let account_keys = build_account_keys(tx_entry, meta);

        // 1. Detect native SOL transfers via preBalances/postBalances diff
        let sol_transfers = detect_sol_transfers(
            &account_keys,
            &meta.pre_balances,
            &meta.post_balances,
            chain_id,
            &signature,
            slot,
            block_time,
        );
        transfers.extend(sol_transfers);

        // 2. Detect SPL token transfers via preTokenBalances/postTokenBalances diff
        let spl_transfers = detect_spl_transfers(
            &account_keys,
            &meta.pre_token_balances,
            &meta.post_token_balances,
            chain_id,
            &signature,
            slot,
            block_time,
        );
        transfers.extend(spl_transfers);
    }

    transfers
}

/// Build ordered list of account pubkeys for a transaction.
/// Versioned txs (v0) append loadedAddresses to the static keys.
fn build_account_keys(
    tx_entry: &SolanaTransactionEntry,
    meta: &SolanaTransactionMeta,
) -> Vec<String> {
    let mut keys: Vec<String> = tx_entry
        .transaction
        .message
        .account_keys
        .iter()
        .map(|k| k.to_pubkey().to_string())
        .collect();

    // Append loaded addresses for versioned transactions
    if let Some(loaded) = &meta.loaded_addresses {
        keys.extend(loaded.writable.iter().cloned());
        keys.extend(loaded.readonly.iter().cloned());
    }

    keys
}

/// Detect native SOL transfers by diffing preBalances and postBalances.
///
/// Sender = account with largest negative delta (excluding fee-only changes).
/// Each account with a positive delta is a receiver.
fn detect_sol_transfers(
    account_keys: &[String],
    pre_balances: &[u64],
    post_balances: &[u64],
    chain_id: u32,
    signature: &str,
    slot: u64,
    block_time: u64,
) -> Vec<Transfer> {
    let mut transfers = Vec::new();
    let n = pre_balances.len().min(post_balances.len()).min(account_keys.len());
    if n == 0 {
        return transfers;
    }

    // Compute deltas (i128 for sign + large values)
    let mut senders: Vec<(usize, i128)> = Vec::new();
    let mut receivers: Vec<(usize, i128)> = Vec::new();

    for i in 0..n {
        let delta = post_balances[i] as i128 - pre_balances[i] as i128;
        if delta < 0 {
            senders.push((i, delta));
        } else if delta > 0 {
            receivers.push((i, delta));
        }
    }

    if senders.is_empty() || receivers.is_empty() {
        return transfers;
    }

    // Primary sender = largest outflow
    let from_idx = senders
        .iter()
        .min_by_key(|(_, d)| *d)
        .map(|(idx, _)| *idx)
        .unwrap();
    let from_addr = &account_keys[from_idx];

    let mut log_index: u32 = 0;
    for (idx, delta) in &receivers {
        transfers.push(Transfer {
            chain_id,
            tx_hash: signature.to_string(),
            log_index,
            token: "sol".to_string(),
            from_addr: from_addr.clone(),
            to_addr: account_keys[*idx].clone(),
            value: delta.to_string(), // lamports
            block_number: slot,
            block_timestamp: block_time,
            swap_type: None,
            status: None,
        });
        log_index += 1;
    }

    transfers
}

/// Detect SPL token transfers by diffing preTokenBalances and postTokenBalances.
///
/// Groups by mint, computes (owner, mint) → delta, then pairs senders to receivers.
/// Uses log_index starting at 10000 to avoid collision with SOL transfer indices.
fn detect_spl_transfers(
    account_keys: &[String],
    pre_token_balances: &[TokenBalance],
    post_token_balances: &[TokenBalance],
    chain_id: u32,
    signature: &str,
    slot: u64,
    block_time: u64,
) -> Vec<Transfer> {
    let mut transfers = Vec::new();

    // Build (owner, mint) → raw_amount maps
    let mut pre_map: HashMap<(String, String), u128> = HashMap::new();
    let mut post_map: HashMap<(String, String), u128> = HashMap::new();

    for tb in pre_token_balances {
        let owner = tb
            .owner
            .clone()
            .unwrap_or_else(|| {
                account_keys
                    .get(tb.account_index)
                    .map(|s| s.clone())
                    .unwrap_or_default()
            });
        let amount: u128 = tb.ui_token_amount.amount.parse().unwrap_or(0);
        pre_map.insert((owner, tb.mint.clone()), amount);
    }

    for tb in post_token_balances {
        let owner = tb
            .owner
            .clone()
            .unwrap_or_else(|| {
                account_keys
                    .get(tb.account_index)
                    .map(|s| s.clone())
                    .unwrap_or_default()
            });
        let amount: u128 = tb.ui_token_amount.amount.parse().unwrap_or(0);
        post_map.insert((owner, tb.mint.clone()), amount);
    }

    // Collect all keys and compute deltas grouped by mint
    let all_keys: HashSet<(String, String)> = pre_map
        .keys()
        .chain(post_map.keys())
        .cloned()
        .collect();

    let mut by_mint: HashMap<String, Vec<(String, i128)>> = HashMap::new();
    for (owner, mint) in all_keys {
        let pre_amt = *pre_map.get(&(owner.clone(), mint.clone())).unwrap_or(&0) as i128;
        let post_amt = *post_map.get(&(owner.clone(), mint.clone())).unwrap_or(&0) as i128;
        let delta = post_amt - pre_amt;
        if delta != 0 {
            by_mint.entry(mint).or_default().push((owner, delta));
        }
    }

    // SPL log_index starts at 10000 to avoid collision with SOL indices
    let mut spl_log_index: u32 = 10000;

    for (mint, deltas) in &by_mint {
        let senders: Vec<&(String, i128)> = deltas.iter().filter(|(_, d)| *d < 0).collect();
        let receivers: Vec<&(String, i128)> = deltas.iter().filter(|(_, d)| *d > 0).collect();

        if receivers.is_empty() {
            continue;
        }

        // Primary sender (largest outflow)
        let from_addr = senders
            .iter()
            .min_by_key(|(_, d)| *d)
            .map(|(owner, _)| owner.clone())
            .unwrap_or_else(|| "unknown".to_string());

        for (to_owner, amount) in &receivers {
            transfers.push(Transfer {
                chain_id,
                tx_hash: signature.to_string(),
                log_index: spl_log_index,
                token: mint.clone(),
                from_addr: from_addr.clone(),
                to_addr: to_owner.clone(),
                value: amount.to_string(),
                block_number: slot,
                block_timestamp: block_time,
                swap_type: None,
                status: None,
            });
            spl_log_index += 1;
        }
    }

    transfers
}
