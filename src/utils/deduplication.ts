import { RedisCache } from '../cache/redis';

/**
 * Event deduplication to prevent processing same event multiple times
 * Important for scenarios with reconnections and backfilling
 */
export class EventDeduplicator {
  private cache: RedisCache;
  private readonly dedupPrefix = 'dedup';
  private readonly dedupTTL = 86400 * 2; // 2 days (longer than cache TTL)

  constructor(cache: RedisCache) {
    this.cache = cache;
  }

  /**
   * Check if event was already processed
   */
  async isDuplicate(
    type: 'erc20' | 'native',
    chainId: number,
    txHash: string,
    additionalKey?: string
  ): Promise<boolean> {
    const key = this.getDedupKey(type, chainId, txHash, additionalKey);

    try {
      const exists = await (this.cache as any).client.exists(key);
      return exists === 1;
    } catch (error) {
      console.error('[Dedup] Error checking duplicate:', error);
      // On error, assume not duplicate to avoid losing events
      return false;
    }
  }

  /**
   * Mark event as processed
   */
  async markAsProcessed(
    type: 'erc20' | 'native',
    chainId: number,
    txHash: string,
    additionalKey?: string
  ): Promise<void> {
    const key = this.getDedupKey(type, chainId, txHash, additionalKey);

    try {
      await (this.cache as any).client.setEx(key, this.dedupTTL, '1');
    } catch (error) {
      console.error('[Dedup] Error marking as processed:', error);
    }
  }

  /**
   * Process event with deduplication check
   */
  async processWithDedup<T>(
    type: 'erc20' | 'native',
    chainId: number,
    txHash: string,
    additionalKey: string | undefined,
    processFn: () => Promise<T>
  ): Promise<{ processed: boolean; result?: T }> {
    // Check if already processed
    const isDup = await this.isDuplicate(type, chainId, txHash, additionalKey);

    if (isDup) {
      console.log(`[Dedup] Skipping duplicate ${type} event: ${txHash}`);
      return { processed: false };
    }

    // Process the event
    const result = await processFn();

    // Mark as processed
    await this.markAsProcessed(type, chainId, txHash, additionalKey);

    return { processed: true, result };
  }

  private getDedupKey(
    type: 'erc20' | 'native',
    chainId: number,
    txHash: string,
    additionalKey?: string
  ): string {
    if (additionalKey) {
      return `${this.dedupPrefix}:${type}:${chainId}:${txHash}:${additionalKey}`;
    }
    return `${this.dedupPrefix}:${type}:${chainId}:${txHash}`;
  }

  /**
   * Clear old deduplication entries
   */
  async clearOldEntries(): Promise<number> {
    try {
      const keys = await (this.cache as any).client.keys(`${this.dedupPrefix}:*`);
      let cleared = 0;

      for (const key of keys) {
        const ttl = await (this.cache as any).client.ttl(key);
        // Already expired or will expire soon
        if (ttl < 3600) {
          await (this.cache as any).client.del(key);
          cleared++;
        }
      }

      if (cleared > 0) {
        console.log(`[Dedup] Cleared ${cleared} old deduplication entries`);
      }
      return cleared;
    } catch (error) {
      console.error('[Dedup] Error clearing old entries:', error);
      return 0;
    }
  }
}
