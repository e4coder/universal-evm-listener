use crate::types::DecodedBlockRange;
use async_trait::async_trait;

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
}
