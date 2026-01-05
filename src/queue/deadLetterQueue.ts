import { RedisCache } from '../cache/redis';

interface DLQItem {
  id: string;
  type: 'erc20' | 'native';
  chainId: number;
  eventData: any;
  error: string;
  timestamp: number;
  retries: number;
}

/**
 * In-Memory Dead Letter Queue for failed event processing
 * Stores events that couldn't be cached for later retry
 * Uses memory instead of Redis to work even when Redis fails
 */
export class DeadLetterQueue {
  private cache: RedisCache;
  private readonly maxRetries = 3;
  private readonly maxItems = 10000; // Limit memory usage
  private items: Map<string, DLQItem> = new Map();

  constructor(cache: RedisCache) {
    this.cache = cache;
  }

  /**
   * Add failed event to DLQ (in-memory)
   */
  async addToDLQ(
    type: 'erc20' | 'native',
    chainId: number,
    eventData: any,
    error: string
  ): Promise<void> {
    // Limit queue size to prevent memory issues
    if (this.items.size >= this.maxItems) {
      // Remove oldest item
      const oldestKey = this.items.keys().next().value;
      if (oldestKey) {
        this.items.delete(oldestKey);
      }
    }

    const id = `${type}:${chainId}:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    const item: DLQItem = {
      id,
      type,
      chainId,
      eventData,
      error,
      timestamp: Date.now(),
      retries: 0,
    };

    this.items.set(id, item);
    console.log(`[DLQ] Added ${type} event (${this.items.size} items in queue)`);
  }

  /**
   * Get all DLQ items
   */
  async getDLQItems(): Promise<DLQItem[]> {
    return Array.from(this.items.values());
  }

  /**
   * Get DLQ size
   */
  getSize(): number {
    return this.items.size;
  }

  /**
   * Retry processing DLQ items
   */
  async processDLQ(): Promise<{ success: number; failed: number }> {
    const items = await this.getDLQItems();
    let success = 0;
    let failed = 0;

    if (items.length === 0) {
      return { success: 0, failed: 0 };
    }

    console.log(`[DLQ] Processing ${items.length} items...`);

    for (const item of items) {
      try {
        if (item.retries >= this.maxRetries) {
          console.log(`[DLQ] Max retries reached for ${item.id}, removing`);
          this.items.delete(item.id);
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
        this.items.delete(item.id);
        success++;
      } catch (error) {
        // Failed again, increment retry count
        item.retries++;
        this.items.set(item.id, item);
        failed++;
      }
    }

    if (success > 0 || failed > 0) {
      console.log(`[DLQ] Processing complete: ${success} success, ${failed} failed, ${this.items.size} remaining`);
    }
    return { success, failed };
  }

  /**
   * Start automatic DLQ processing
   */
  startAutoProcessing(intervalMs = 30000): void {
    // Every 30 seconds (faster since it's in-memory)
    setInterval(async () => {
      if (this.items.size > 0) {
        await this.processDLQ();
      }
    }, intervalMs);
  }

  /**
   * Clear old DLQ items
   */
  async clearOldItems(olderThanMs = 600000): Promise<number> {
    const cutoff = Date.now() - olderThanMs;
    let cleared = 0;

    for (const [id, item] of this.items) {
      if (item.timestamp < cutoff) {
        this.items.delete(id);
        cleared++;
      }
    }

    if (cleared > 0) {
      console.log(`[DLQ] Cleared ${cleared} old items`);
    }
    return cleared;
  }
}
