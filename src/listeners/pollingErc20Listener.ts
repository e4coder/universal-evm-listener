import { Alchemy } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';
import { NetworkConfig } from '../config/networks';
import { CheckpointManager } from '../persistence/checkpoint';
import { EventDeduplicator } from '../utils/deduplication';
import { DeadLetterQueue } from '../queue/deadLetterQueue';
import { EventMonitor } from '../monitoring/eventMonitor';
import { RateLimiter } from '../utils/rateLimiter';

const ERC20_TRANSFER_EVENT = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';

/**
 * Polling-based ERC20 listener using getLogs
 * More reliable than WebSockets, handles reorgs automatically
 */
export class PollingERC20Listener {
  private alchemy: Alchemy;
  private cache: RedisCache;
  private networkConfig: NetworkConfig;
  private checkpoint: CheckpointManager;
  private deduplicator: EventDeduplicator;
  private dlq: DeadLetterQueue;
  private monitor: EventMonitor;
  private rateLimiter: RateLimiter;

  private lastProcessedBlock = 0;
  private isShuttingDown = false;
  private isProcessing = false;
  private pollingInterval: NodeJS.Timeout | null = null;
  private blockCache: Map<number, { timestamp: number }> = new Map(); // Cache block timestamps

  // Configuration
  private readonly REORG_SAFETY_BLOCKS = 10; // Look back 10 blocks for safety
  private readonly CONFIRMATION_BLOCKS = 3; // Only process blocks older than 3 blocks
  private readonly POLL_INTERVAL_MS = 2000; // Poll every 2 seconds
  private readonly MAX_BLOCKS_PER_QUERY = 100; // Max blocks to query at once
  private readonly BLOCK_CACHE_SIZE = 100; // Keep last 100 blocks cached
  private readonly MAX_BACKFILL_BLOCKS = 500; // Don't backfill more than 500 blocks on startup

  constructor(
    alchemy: Alchemy,
    cache: RedisCache,
    networkConfig: NetworkConfig,
    checkpoint: CheckpointManager,
    deduplicator: EventDeduplicator,
    dlq: DeadLetterQueue,
    monitor: EventMonitor,
    rateLimiter: RateLimiter
  ) {
    this.alchemy = alchemy;
    this.cache = cache;
    this.networkConfig = networkConfig;
    this.checkpoint = checkpoint;
    this.deduplicator = deduplicator;
    this.dlq = dlq;
    this.monitor = monitor;
    this.rateLimiter = rateLimiter;
  }

  async start(): Promise<void> {
    console.log(`[${this.networkConfig.name}] Starting Polling ERC20 Listener...`);

    // Get current block
    const currentBlock = await this.alchemy.core.getBlockNumber();
    const savedCheckpoint = await this.checkpoint.getCheckpoint(this.networkConfig.chainId);

    if (savedCheckpoint) {
      const blocksBehind = currentBlock - savedCheckpoint;

      if (blocksBehind > this.MAX_BACKFILL_BLOCKS) {
        // Checkpoint too old - skip to recent blocks to prevent memory explosion
        const newStart = currentBlock - this.REORG_SAFETY_BLOCKS;
        console.log(
          `[${this.networkConfig.name}] ⚠️ Checkpoint ${savedCheckpoint} is ${blocksBehind} blocks behind (max: ${this.MAX_BACKFILL_BLOCKS}). Skipping to block ${newStart}`
        );
        this.lastProcessedBlock = newStart;
        await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, newStart);
      } else {
        console.log(
          `[${this.networkConfig.name}] Found checkpoint at block ${savedCheckpoint} (${blocksBehind} blocks behind)`
        );
        this.lastProcessedBlock = savedCheckpoint;
      }
    } else {
      // First start - begin from current block minus safety margin
      const startBlock = currentBlock - this.REORG_SAFETY_BLOCKS;
      console.log(
        `[${this.networkConfig.name}] 🆕 First start. Starting from block ${startBlock}`
      );
      this.lastProcessedBlock = startBlock;
      await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, startBlock);
    }

    // Start polling
    this.startPolling();

    console.log(`[${this.networkConfig.name}] ✅ Polling ERC20 Listener active (poll every ${this.POLL_INTERVAL_MS}ms)`);
  }

  private startPolling(): void {
    this.pollingInterval = setInterval(async () => {
      if (this.isShuttingDown || this.isProcessing) {
        return;
      }

      // Skip polling if Redis is unhealthy - prevents DLQ from exploding
      if (!this.cache.isHealthy()) {
        console.log(`[${this.networkConfig.name}] ⏸️ Skipping poll - Redis unhealthy`);
        return;
      }

      try {
        await this.pollForEvents();
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Polling error:`, error);
        this.monitor.recordError(this.networkConfig.chainId);
      }
    }, this.POLL_INTERVAL_MS);
  }

  private async pollForEvents(): Promise<void> {
    this.isProcessing = true;

    try {
      // Get current block
      const currentBlock = await this.alchemy.core.getBlockNumber();

      // Calculate safe block range
      // Process from lastProcessedBlock to (currentBlock - CONFIRMATION_BLOCKS)
      const toBlock = currentBlock - this.CONFIRMATION_BLOCKS;
      const fromBlock = Math.max(
        this.lastProcessedBlock - this.REORG_SAFETY_BLOCKS + 1,
        this.lastProcessedBlock + 1
      );

      // Skip if no new blocks to process
      if (fromBlock > toBlock) {
        return;
      }

      // Limit query size
      const actualToBlock = Math.min(fromBlock + this.MAX_BLOCKS_PER_QUERY - 1, toBlock);

      console.log(
        `[${this.networkConfig.name}] Polling blocks ${fromBlock} to ${actualToBlock} (current: ${currentBlock})`
      );

      // Query Transfer events using getLogs
      const logs = await this.rateLimiter.executeWithLimit(async () => {
        return await this.alchemy.core.getLogs({
          fromBlock,
          toBlock: actualToBlock,
          topics: [ERC20_TRANSFER_EVENT], // Filter for Transfer events only
        });
      });

      console.log(
        `[${this.networkConfig.name}] Found ${logs.length} Transfer events in blocks ${fromBlock}-${actualToBlock}`
      );

      // Process each event
      for (const log of logs) {
        await this.processTransferEvent(log);
      }

      // Update checkpoint
      this.lastProcessedBlock = actualToBlock;
      await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, actualToBlock);

      this.monitor.recordBlockProcessed(this.networkConfig.chainId);
    } finally {
      this.isProcessing = false;
    }
  }

  private async getBlockTimestamp(blockNumber: number): Promise<number> {
    // Check cache first
    const cached = this.blockCache.get(blockNumber);
    if (cached) {
      return cached.timestamp;
    }

    // Fetch block with rate limiting
    const block = await this.rateLimiter.executeWithLimit(async () => {
      return await this.alchemy.core.getBlock(blockNumber);
    });
    const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

    // Cache it (with LRU eviction)
    if (this.blockCache.size >= this.BLOCK_CACHE_SIZE) {
      const firstKey = this.blockCache.keys().next().value;
      if (firstKey !== undefined) {
        this.blockCache.delete(firstKey);
      }
    }
    this.blockCache.set(blockNumber, { timestamp });

    return timestamp;
  }

  private async processTransferEvent(log: any): Promise<void> {
    try {
      // Decode Transfer event
      // Transfer(address indexed from, address indexed to, uint256 value)
      // topics[0] = event signature
      // topics[1] = from (indexed)
      // topics[2] = to (indexed)
      // data = value (not indexed)

      if (log.topics.length < 3) {
        return; // Invalid Transfer event
      }

      const tokenAddress = log.address.toLowerCase();
      const from = '0x' + log.topics[1].slice(26); // Remove padding
      const to = '0x' + log.topics[2].slice(26); // Remove padding
      const value = log.data; // uint256 as hex string
      const txHash = log.transactionHash;
      const blockNumber = log.blockNumber;

      // Check for duplicates
      const isDuplicate = await this.deduplicator.isDuplicate(
        'erc20',
        this.networkConfig.chainId,
        txHash,
        tokenAddress
      );

      if (isDuplicate) {
        return;
      }

      // Get block timestamp (with caching - major performance boost!)
      const timestamp = await this.getBlockTimestamp(blockNumber);

      // Store in Redis
      try {
        await this.cache.storeERC20Transfer(
          this.networkConfig.chainId,
          txHash,
          tokenAddress,
          from,
          to,
          value,
          blockNumber,
          timestamp
        );

        await this.deduplicator.markAsProcessed(
          'erc20',
          this.networkConfig.chainId,
          txHash,
          tokenAddress
        );

        this.monitor.recordERC20Event(this.networkConfig.chainId, false);
      } catch (cacheError: any) {
        console.error(
          `[${this.networkConfig.name}] Cache error, adding to DLQ:`,
          cacheError.message
        );

        await this.dlq.addToDLQ(
          'erc20',
          this.networkConfig.chainId,
          {
            chainId: this.networkConfig.chainId,
            txHash,
            token: tokenAddress,
            from,
            to,
            value,
            blockNumber,
            timestamp,
          },
          cacheError.message
        );

        this.monitor.recordError(this.networkConfig.chainId);
      }
    } catch (error: any) {
      console.error(
        `[${this.networkConfig.name}] Error processing transfer event:`,
        error
      );
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  stop(): void {
    this.isShuttingDown = true;
    if (this.pollingInterval) {
      clearInterval(this.pollingInterval);
    }
    console.log(`[${this.networkConfig.name}] Polling ERC20 Listener stopped`);
  }
}
