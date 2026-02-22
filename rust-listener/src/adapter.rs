use crate::types::{DecodedBlockRange, MempoolUpdate};
use async_trait::async_trait;
use std::collections::HashSet;

/// Protocol-agnostic chain adapter trait.
/// Each blockchain protocol (EVM, Tron, Solana, Bitcoin) implements this trait.
/// The generic pipeline (fetcher_loop, processor_loop, checkpointing) works
/// exclusively with this interface — no protocol-specific knowledge needed.
#[async_trait]
pub trait ChainAdapter: Send + Sync + 'static {
    /// Get the current chain tip block number.
    async fn get_block_number(&self) -> Result<u64, String>;

    /// Fetch and decode all events in a block range.
    /// Returns fully decoded, normalized events with timestamps resolved.
    /// Empty Vecs are valid for unsupported event types on a given chain.
    async fn fetch_decoded(&self, from_block: u64, to_block: u64) -> Result<DecodedBlockRange, String>;

    /// Whether this adapter supports mempool monitoring (default: false).
    /// When true, the poller will spawn a mempool_loop alongside the block fetcher.
    fn supports_mempool(&self) -> bool {
        false
    }

    /// Poll the mempool for new/removed transactions.
    /// `known_txids` is the set of txids seen in the previous poll cycle.
    /// Returns a delta: new pending transfers, confirmed txids, dropped txids.
    async fn poll_mempool(&self, _known_txids: &HashSet<String>) -> Result<MempoolUpdate, String> {
        Err("Mempool not supported".into())
    }
}
