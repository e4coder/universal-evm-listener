import { RedisCache } from '../cache/redis';

/**
 * Checkpoint manager for tracking last processed block per network
 * Enables resuming from last known position after restart
 */
export class CheckpointManager {
  private cache: RedisCache;
  private checkpointPrefix = 'checkpoint';

  constructor(cache: RedisCache) {
    this.cache = cache;
  }

  /**
   * Save checkpoint for a specific chain
   */
  async saveCheckpoint(chainId: number, blockNumber: number): Promise<void> {
    const key = `${this.checkpointPrefix}:${chainId}`;
    // Store as string with no expiration (persistent)
    await (this.cache as any).client.set(key, blockNumber.toString());
  }

  /**
   * Get last checkpoint for a chain
   */
  async getCheckpoint(chainId: number): Promise<number | null> {
    const key = `${this.checkpointPrefix}:${chainId}`;
    const value = await (this.cache as any).client.get(key);
    return value ? parseInt(value, 10) : null;
  }

  /**
   * Get starting block: use checkpoint if exists, otherwise current block
   */
  async getStartingBlock(chainId: number, currentBlock: number): Promise<number> {
    const checkpoint = await this.getCheckpoint(chainId);

    if (checkpoint) {
      console.log(`[Chain ${chainId}] Resuming from checkpoint: block ${checkpoint}`);
      return checkpoint;
    }

    console.log(`[Chain ${chainId}] No checkpoint found, starting from current block ${currentBlock}`);
    return currentBlock;
  }

  /**
   * Save checkpoint with batching (every N blocks to reduce Redis writes)
   */
  private lastSavedBlock: { [chainId: number]: number } = {};
  private readonly CHECKPOINT_INTERVAL = 10; // Save every 10 blocks

  async saveCheckpointBatched(chainId: number, blockNumber: number): Promise<void> {
    const lastSaved = this.lastSavedBlock[chainId] || 0;

    if (blockNumber - lastSaved >= this.CHECKPOINT_INTERVAL) {
      await this.saveCheckpoint(chainId, blockNumber);
      this.lastSavedBlock[chainId] = blockNumber;
    }
  }
}
