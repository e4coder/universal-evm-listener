import { Alchemy } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';
import { NetworkConfig } from '../config/networks';
import { CheckpointManager } from '../persistence/checkpoint';
import { EventDeduplicator } from '../utils/deduplication';
import { DeadLetterQueue } from '../queue/deadLetterQueue';
import { EventMonitor } from '../monitoring/eventMonitor';
import { RateLimiter } from '../utils/rateLimiter';

const ERC20_TRANSFER_EVENT = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';

export class SmartReliableERC20Listener {
  private alchemy: Alchemy;
  private cache: RedisCache;
  private networkConfig: NetworkConfig;
  private checkpoint: CheckpointManager;
  private deduplicator: EventDeduplicator;
  private dlq: DeadLetterQueue;
  private monitor: EventMonitor;
  private rateLimiter: RateLimiter;

  private reconnectAttempts = 0;
  private maxReconnectAttempts = 10;
  private lastProcessedBlock = 0;
  private isShuttingDown = false;
  private isBackfilling = false; // Prevent concurrent backfills
  private lastWebSocketBlockTime = Date.now(); // Track last WebSocket block event

  // Configuration
  private readonly MAX_BACKFILL_BLOCKS = 100; // Don't backfill more than this at once (reduced for free tier)
  private readonly BACKFILL_CHUNK_SIZE = 10; // Process in chunks (reduced for free tier)

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
    console.log(`[${this.networkConfig.name}] Starting Smart Reliable ERC20 Listener...`);

    const currentBlock = await this.alchemy.core.getBlockNumber();
    const savedCheckpoint = await this.checkpoint.getCheckpoint(this.networkConfig.chainId);

    if (savedCheckpoint) {
      // NOT first start - we have a checkpoint
      console.log(
        `[${this.networkConfig.name}] Found checkpoint at block ${savedCheckpoint} (current: ${currentBlock})`
      );

      const gap = currentBlock - savedCheckpoint;

      if (gap > this.MAX_BACKFILL_BLOCKS) {
        console.warn(
          `[${this.networkConfig.name}] ⚠️  Gap too large (${gap} blocks). Limiting backfill to ${this.MAX_BACKFILL_BLOCKS} blocks.`
        );
        console.warn(
          `[${this.networkConfig.name}] Starting from block ${currentBlock - this.MAX_BACKFILL_BLOCKS} instead of ${savedCheckpoint}`
        );

        this.lastProcessedBlock = currentBlock - this.MAX_BACKFILL_BLOCKS;

        // Update checkpoint to new starting point
        await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, this.lastProcessedBlock);
      } else if (gap > 0) {
        console.log(`[${this.networkConfig.name}] Backfilling ${gap} blocks in background...`);
        // Start from current block immediately, backfill in background
        this.lastProcessedBlock = currentBlock;

        // Queue startup backfill asynchronously
        this.queueBackfill(savedCheckpoint + 1, currentBlock).catch((error: any) => {
          console.error(`[${this.networkConfig.name}] Startup backfill failed:`, error);
          this.monitor.recordError(this.networkConfig.chainId);
        });
      } else {
        this.lastProcessedBlock = savedCheckpoint;
      }
    } else {
      // FIRST start - no checkpoint exists
      console.log(
        `[${this.networkConfig.name}] 🆕 First start detected. Starting from current block ${currentBlock} (no backfill)`
      );
      this.lastProcessedBlock = currentBlock;

      // Save initial checkpoint
      await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, currentBlock);
    }

    await this.setupWebSocketListener();
    this.setupConnectionMonitoring();
    this.setupPeriodicSync();
  }

  private async setupWebSocketListener(): Promise<void> {
    try {
      // Listen to block events to track progress
      this.alchemy.ws.on('block', async (blockNumber: number) => {
        // Track that WebSocket is alive
        this.lastWebSocketBlockTime = Date.now();

        // Check if we missed any blocks
        if (blockNumber > this.lastProcessedBlock + 1 && this.lastProcessedBlock > 0) {
          const missedBlocks = blockNumber - this.lastProcessedBlock - 1;

          if (missedBlocks > this.MAX_BACKFILL_BLOCKS) {
            console.error(
              `[${this.networkConfig.name}] ⚠️  TOO MANY missed blocks (${missedBlocks})! Skipping to current.`
            );
            this.monitor.recordMissedBlocks(this.networkConfig.chainId, missedBlocks);
            this.lastProcessedBlock = blockNumber - 100; // Start 100 blocks back
          } else if (!this.isBackfilling) {
            // Queue backfill asynchronously - don't block new block processing
            console.warn(
              `[${this.networkConfig.name}] Detected ${missedBlocks} missed block(s). Queueing backfill...`
            );
            this.monitor.recordMissedBlocks(this.networkConfig.chainId, missedBlocks);

            const fromBlock = this.lastProcessedBlock + 1;
            const toBlock = blockNumber - 1;

            // Fire and forget - don't await
            this.queueBackfill(fromBlock, toBlock).catch((error: any) => {
              console.error(`[${this.networkConfig.name}] Background backfill failed:`, error);
              this.monitor.recordError(this.networkConfig.chainId);
            });
          }
        }

        // Always update to current block immediately (don't wait for backfill)
        this.lastProcessedBlock = blockNumber;
        this.monitor.recordBlockProcessed(this.networkConfig.chainId);

        // Save checkpoint every N blocks
        await this.checkpoint.saveCheckpointBatched(this.networkConfig.chainId, blockNumber);
      });

      // Note: Instead of listening to ALL mined transactions (which overwhelms free tier),
      // we rely on the block listener to detect new blocks and periodically backfill
      // This is much more efficient for the Alchemy free tier

      // Handle WebSocket errors
      this.alchemy.ws.on('error', (error) => {
        console.error(`[${this.networkConfig.name}] WebSocket error:`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      });

      // Handle WebSocket close
      this.alchemy.ws.on('close', () => {
        console.warn(`[${this.networkConfig.name}] WebSocket connection closed`);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      });

      console.log(`[${this.networkConfig.name}] Smart Reliable ERC20 Listener active`);
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error setting up WebSocket:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
      if (!this.isShuttingDown) {
        this.handleDisconnection();
      }
    }
  }

  private async processTransaction(tx: any, isBackfill = false): Promise<void> {
    try {
      if (tx.hash) {
        const receipt = await this.rateLimiter.executeWithLimit(async () => {
          return await this.alchemy.core.getTransactionReceipt(tx.hash);
        });

        if (receipt && receipt.logs) {
          for (const log of receipt.logs) {
            if (log.topics[0] === ERC20_TRANSFER_EVENT && log.topics.length === 3) {
              await this.handleTransferEvent(log, receipt.blockNumber, isBackfill);
            }
          }
        }
      }
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error processing transaction:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private async handleTransferEvent(log: any, blockNumber: number, isBackfill = false): Promise<void> {
    try {
      // Check for duplicates
      const isDuplicate = await this.deduplicator.isDuplicate(
        'erc20',
        this.networkConfig.chainId,
        log.transactionHash,
        log.address
      );

      if (isDuplicate) {
        return; // Skip duplicate
      }

      const from = '0x' + log.topics[1].slice(26);
      const to = '0x' + log.topics[2].slice(26);
      const value = log.data;

      // Get block with rate limiting
      const block = await this.rateLimiter.executeWithLimit(async () => {
        return await this.alchemy.core.getBlock(blockNumber);
      });
      const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

      try {
        // Try to cache
        await this.cache.storeERC20Transfer(
          this.networkConfig.chainId,
          log.transactionHash,
          log.address,
          from,
          to,
          value,
          blockNumber,
          timestamp
        );

        // Mark as processed
        await this.deduplicator.markAsProcessed(
          'erc20',
          this.networkConfig.chainId,
          log.transactionHash,
          log.address
        );

        // Record metrics
        this.monitor.recordERC20Event(this.networkConfig.chainId, isBackfill);

        if (!isBackfill) {
          console.log(
            `[${this.networkConfig.name}] ERC20 Transfer cached: ${from} -> ${to} (Token: ${log.address})`
          );
        }
      } catch (cacheError: any) {
        // Cache failed - add to DLQ
        console.error(`[${this.networkConfig.name}] Cache error, adding to DLQ:`, cacheError.message);

        await this.dlq.addToDLQ(
          'erc20',
          this.networkConfig.chainId,
          {
            chainId: this.networkConfig.chainId,
            txHash: log.transactionHash,
            token: log.address,
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
      console.error(`[${this.networkConfig.name}] Error handling Transfer event:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private async queueBackfill(fromBlock: number, toBlock: number): Promise<void> {
    // Acquire lock
    if (this.isBackfilling) {
      console.log(
        `[${this.networkConfig.name}] Backfill already in progress, skipping blocks ${fromBlock}-${toBlock}`
      );
      return;
    }

    this.isBackfilling = true;

    try {
      await this.backfillBlocks(fromBlock, toBlock);
      // Update checkpoint after successful backfill
      await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, toBlock);
    } finally {
      this.isBackfilling = false;
    }
  }

  private async backfillBlocks(fromBlock: number, toBlock: number): Promise<void> {
    try {
      const totalBlocks = toBlock - fromBlock + 1;
      console.log(
        `[${this.networkConfig.name}] Backfilling ${totalBlocks} blocks (${fromBlock} to ${toBlock})...`
      );

      let totalEvents = 0;

      // Process in chunks
      for (let start = fromBlock; start <= toBlock; start += this.BACKFILL_CHUNK_SIZE) {
        const end = Math.min(start + this.BACKFILL_CHUNK_SIZE - 1, toBlock);

        try {
          const logs = await this.rateLimiter.executeWithLimit(async () => {
            return await this.alchemy.core.getLogs({
              fromBlock: start,
              toBlock: end,
              topics: [ERC20_TRANSFER_EVENT],
            });
          });

          console.log(
            `[${this.networkConfig.name}] Backfill chunk ${start}-${end}: found ${logs.length} transfers`
          );

          for (const log of logs) {
            if (log.topics.length === 3) {
              await this.handleTransferEvent(log, log.blockNumber || start, true);
              totalEvents++;
            }
          }

          // Delay between chunks to avoid rate limiting
          await new Promise((resolve) => setTimeout(resolve, 1000));
        } catch (error) {
          console.error(
            `[${this.networkConfig.name}] Error backfilling chunk ${start}-${end}:`,
            error
          );
          this.monitor.recordError(this.networkConfig.chainId);
          // Continue with next chunk
        }
      }

      console.log(
        `[${this.networkConfig.name}] ✅ Backfill complete: ${totalEvents} ERC20 transfers cached`
      );
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error during backfill:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private handleDisconnection(): void {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      console.error(
        `[${this.networkConfig.name}] ❌ Max reconnection attempts reached. Manual intervention required.`
      );
      return;
    }

    this.reconnectAttempts++;
    this.monitor.recordReconnection(this.networkConfig.chainId);

    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);

    console.log(
      `[${this.networkConfig.name}] Reconnecting in ${delay / 1000}s (attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts})...`
    );

    setTimeout(async () => {
      try {
        await this.setupWebSocketListener();
        this.reconnectAttempts = 0;
        console.log(`[${this.networkConfig.name}] ✅ Reconnected successfully`);
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Reconnection failed:`, error);
        this.handleDisconnection();
      }
    }, delay);
  }

  private setupConnectionMonitoring(): void {
    setInterval(async () => {
      try {
        if (!this.isShuttingDown) {
          // Check if WebSocket has been silent for too long (2 minutes)
          const timeSinceLastBlock = Date.now() - this.lastWebSocketBlockTime;
          if (timeSinceLastBlock > 120000) {
            console.error(
              `[${this.networkConfig.name}] ⚠️  WebSocket dead: No block events for ${Math.floor(timeSinceLastBlock / 1000)}s. Reconnecting...`
            );
            this.monitor.recordError(this.networkConfig.chainId);
            this.handleDisconnection();
            return;
          }

          const blockNumber = await this.alchemy.core.getBlockNumber();
          if (blockNumber < this.lastProcessedBlock) {
            console.warn(
              `[${this.networkConfig.name}] ⚠️  Block number went backwards. Possible reorg or connection issue.`
            );
          }
        }
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Connection check failed:`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      }
    }, 30000);
  }

  private setupPeriodicSync(): void {
    // Poll for new blocks every 15 seconds and backfill any gaps
    setInterval(async () => {
      try {
        if (!this.isShuttingDown && !this.isBackfilling) {
          const currentBlock = await this.alchemy.core.getBlockNumber();

          // If we've fallen behind, backfill the gap
          if (currentBlock > this.lastProcessedBlock + 1) {
            const gap = currentBlock - this.lastProcessedBlock;

            if (gap > 5) { // Only backfill if gap is significant
              this.isBackfilling = true; // Set lock
              console.log(
                `[${this.networkConfig.name}] Syncing ${gap} blocks (${this.lastProcessedBlock + 1} to ${currentBlock})...`
              );

              try {
                await this.backfillBlocks(this.lastProcessedBlock + 1, currentBlock);
                this.lastProcessedBlock = currentBlock;
                await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, currentBlock);
              } finally {
                this.isBackfilling = false; // Release lock
              }
            }
          }
        }
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Periodic sync failed:`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        this.isBackfilling = false; // Release lock on error
      }
    }, 15000); // Check every 15 seconds
  }

  stop(): void {
    this.isShuttingDown = true;
    this.alchemy.ws.removeAllListeners();
    console.log(`[${this.networkConfig.name}] Smart Reliable ERC20 Listener stopped`);
  }
}
