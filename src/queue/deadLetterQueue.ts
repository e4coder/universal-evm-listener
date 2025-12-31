import { RedisCache } from '../cache/redis';

/**
 * Dead Letter Queue for failed event processing
 * Stores events that couldn't be cached for later retry
 */
export class DeadLetterQueue {
  private cache: RedisCache;
  private readonly dlqPrefix = 'dlq';
  private readonly maxRetries = 3;

  constructor(cache: RedisCache) {
    this.cache = cache;
  }

  /**
   * Add failed event to DLQ
   */
  async addToDLQ(
    type: 'erc20' | 'native',
    chainId: number,
    eventData: any,
    error: string
  ): Promise<void> {
    const key = `${this.dlqPrefix}:${type}:${chainId}:${Date.now()}`;
    const value = JSON.stringify({
      type,
      chainId,
      eventData,
      error,
      timestamp: Date.now(),
      retries: 0,
    });

    try {
      await (this.cache as any).client.setEx(key, 86400 * 7, value); // 7 days TTL
      console.log(`[DLQ] Added ${type} event to dead letter queue: ${key}`);
    } catch (err) {
      console.error('[DLQ] Failed to add to dead letter queue:', err);
    }
  }

  /**
   * Get all DLQ items
   */
  async getDLQItems(): Promise<any[]> {
    try {
      const keys = await (this.cache as any).client.keys(`${this.dlqPrefix}:*`);
      const items: any[] = [];

      for (const key of keys) {
        const value = await (this.cache as any).client.get(key);
        if (value) {
          items.push({
            key,
            ...JSON.parse(value),
          });
        }
      }

      return items;
    } catch (error) {
      console.error('[DLQ] Error getting DLQ items:', error);
      return [];
    }
  }

  /**
   * Retry processing DLQ items
   */
  async processDLQ(): Promise<{ success: number; failed: number }> {
    const items = await this.getDLQItems();
    let success = 0;
    let failed = 0;

    console.log(`[DLQ] Processing ${items.length} items from dead letter queue...`);

    for (const item of items) {
      try {
        if (item.retries >= this.maxRetries) {
          console.log(`[DLQ] Max retries reached for ${item.key}, skipping`);
          failed++;
          continue;
        }

        // Try to reprocess
        if (item.type === 'erc20') {
          await this.cache.storeERC20Transfer(
            item.eventData.chainId,
            item.eventData.txHash,
            item.eventData.token,
            item.eventData.from,
            item.eventData.to,
            item.eventData.value,
            item.eventData.blockNumber,
            item.eventData.timestamp
          );
        } else if (item.type === 'native') {
          await this.cache.storeNativeTransfer(
            item.eventData.chainId,
            item.eventData.txHash,
            item.eventData.from,
            item.eventData.to,
            item.eventData.value,
            item.eventData.blockNumber,
            item.eventData.timestamp
          );
        }

        // Success! Remove from DLQ
        await (this.cache as any).client.del(item.key);
        success++;
        console.log(`[DLQ] Successfully reprocessed ${item.key}`);
      } catch (error) {
        // Failed again, increment retry count
        item.retries++;
        await (this.cache as any).client.setEx(item.key, 86400 * 7, JSON.stringify(item));
        failed++;
        console.error(`[DLQ] Failed to reprocess ${item.key}:`, error);
      }
    }

    console.log(`[DLQ] Processing complete: ${success} success, ${failed} failed`);
    return { success, failed };
  }

  /**
   * Start automatic DLQ processing
   */
  startAutoProcessing(intervalMs = 300000): void {
    // Every 5 minutes
    setInterval(async () => {
      const items = await this.getDLQItems();
      if (items.length > 0) {
        console.log(`[DLQ] Auto-processing ${items.length} items...`);
        await this.processDLQ();
      }
    }, intervalMs);
  }

  /**
   * Clear old DLQ items
   */
  async clearOldItems(olderThanDays = 7): Promise<number> {
    const items = await this.getDLQItems();
    const cutoff = Date.now() - olderThanDays * 86400 * 1000;
    let cleared = 0;

    for (const item of items) {
      if (item.timestamp < cutoff) {
        await (this.cache as any).client.del(item.key);
        cleared++;
      }
    }

    console.log(`[DLQ] Cleared ${cleared} old items`);
    return cleared;
  }
}
