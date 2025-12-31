import { Alchemy } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';
import { NetworkConfig } from '../config/networks';
import { CheckpointManager } from '../persistence/checkpoint';
import { EventDeduplicator } from '../utils/deduplication';
import { DeadLetterQueue } from '../queue/deadLetterQueue';
import { EventMonitor } from '../monitoring/eventMonitor';
import { RateLimiter } from '../utils/rateLimiter';

export class SmartReliableNativeListener {
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

  private readonly MAX_BACKFILL_BLOCKS = 50; // Smaller for native (reduced for free tier)
  private readonly BACKFILL_CHUNK_SIZE = 10;

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
    console.log(`[${this.networkConfig.name}] Starting Smart Reliable Native Listener...`);

    const currentBlock = await this.alchemy.core.getBlockNumber();
    const checkpointKey = `${this.networkConfig.chainId}_native`; // Separate checkpoint for native
    const savedCheckpoint = await this.checkpoint.getCheckpoint(parseInt(checkpointKey));

    if (savedCheckpoint) {
      console.log(
        `[${this.networkConfig.name}] Found native checkpoint at block ${savedCheckpoint}`
      );

      const gap = currentBlock - savedCheckpoint;

      if (gap > this.MAX_BACKFILL_BLOCKS) {
        console.warn(
          `[${this.networkConfig.name}] ⚠️  Native gap too large (${gap} blocks). Limiting backfill.`
        );
        this.lastProcessedBlock = currentBlock - this.MAX_BACKFILL_BLOCKS;
        await this.checkpoint.saveCheckpoint(parseInt(checkpointKey), this.lastProcessedBlock);
      } else if (gap > 0) {
        console.log(`[${this.networkConfig.name}] Backfilling ${gap} blocks (native)...`);
        this.lastProcessedBlock = savedCheckpoint;
        await this.backfillBlocks(savedCheckpoint + 1, currentBlock);
      } else {
        this.lastProcessedBlock = savedCheckpoint;
      }
    } else {
      console.log(
        `[${this.networkConfig.name}] 🆕 First start (native). Starting from current block ${currentBlock}`
      );
      this.lastProcessedBlock = currentBlock;
      await this.checkpoint.saveCheckpoint(parseInt(checkpointKey), currentBlock);
    }

    await this.setupWebSocketListener();
    this.setupConnectionMonitoring();
    this.setupPeriodicSync();
  }

  private async setupWebSocketListener(): Promise<void> {
    try {
      const checkpointKey = `${this.networkConfig.chainId}_native`;

      this.alchemy.ws.on('block', async (blockNumber: number) => {
        if (blockNumber > this.lastProcessedBlock + 1 && this.lastProcessedBlock > 0) {
          const missedBlocks = blockNumber - this.lastProcessedBlock - 1;

          if (missedBlocks > this.MAX_BACKFILL_BLOCKS) {
            console.error(
              `[${this.networkConfig.name}] ⚠️  Too many missed native blocks (${missedBlocks})!`
            );
            this.monitor.recordMissedBlocks(this.networkConfig.chainId, missedBlocks);
            this.lastProcessedBlock = blockNumber - 50;
          } else {
            console.warn(
              `[${this.networkConfig.name}] Detected ${missedBlocks} missed native blocks. Backfilling...`
            );
            this.monitor.recordMissedBlocks(this.networkConfig.chainId, missedBlocks);
            await this.backfillBlocks(this.lastProcessedBlock + 1, blockNumber - 1);
          }
        }

        this.lastProcessedBlock = blockNumber;
        this.monitor.recordBlockProcessed(this.networkConfig.chainId);
        await this.checkpoint.saveCheckpointBatched(parseInt(checkpointKey), blockNumber);
      });

      // Note: Instead of listening to ALL mined transactions (which overwhelms free tier),
      // we rely on the block listener to detect new blocks and periodically backfill
      // This is much more efficient for the Alchemy free tier

      this.alchemy.ws.on('error', (error) => {
        console.error(`[${this.networkConfig.name}] WebSocket error (native):`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      });

      this.alchemy.ws.on('close', () => {
        console.warn(`[${this.networkConfig.name}] WebSocket closed (native)`);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      });

      console.log(`[${this.networkConfig.name}] Smart Reliable Native Listener active`);
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error setting up WebSocket (native):`, error);
      this.monitor.recordError(this.networkConfig.chainId);
      if (!this.isShuttingDown) {
        this.handleDisconnection();
      }
    }
  }

  private async processTransaction(tx: any, isBackfill = false): Promise<void> {
    try {
      if (tx.transaction && tx.transaction.value && tx.transaction.value !== '0x0') {
        await this.handleNativeTransfer(tx.transaction, isBackfill);
      }
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error processing native transfer:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private async handleNativeTransfer(tx: any, isBackfill = false): Promise<void> {
    try {
      const from = tx.from;
      const to = tx.to;
      const value = tx.value;
      const txHash = tx.hash;

      if (!to) return;

      // Check for duplicates
      const isDuplicate = await this.deduplicator.isDuplicate(
        'native',
        this.networkConfig.chainId,
        txHash
      );

      if (isDuplicate) {
        return;
      }

      const receipt = await this.rateLimiter.executeWithLimit(async () => {
        return await this.alchemy.core.getTransactionReceipt(txHash);
      });

      if (!receipt) return;

      const blockNumber = receipt.blockNumber;
      const block = await this.rateLimiter.executeWithLimit(async () => {
        return await this.alchemy.core.getBlock(blockNumber);
      });
      const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

      try {
        await this.cache.storeNativeTransfer(
          this.networkConfig.chainId,
          txHash,
          from,
          to,
          value,
          blockNumber,
          timestamp
        );

        await this.deduplicator.markAsProcessed('native', this.networkConfig.chainId, txHash);
        this.monitor.recordNativeEvent(this.networkConfig.chainId, isBackfill);

        if (!isBackfill) {
          console.log(
            `[${this.networkConfig.name}] Native Transfer cached: ${from} -> ${to} (${this.networkConfig.nativeSymbol})`
          );
        }
      } catch (cacheError: any) {
        console.error(`[${this.networkConfig.name}] Cache error (native), adding to DLQ:`, cacheError.message);

        await this.dlq.addToDLQ(
          'native',
          this.networkConfig.chainId,
          {
            chainId: this.networkConfig.chainId,
            txHash,
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
      console.error(`[${this.networkConfig.name}] Error handling native transfer:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private async backfillBlocks(fromBlock: number, toBlock: number): Promise<void> {
    try {
      const totalBlocks = toBlock - fromBlock + 1;
      console.log(
        `[${this.networkConfig.name}] Backfilling ${totalBlocks} blocks for native transfers...`
      );

      let totalTransfers = 0;

      for (let block = fromBlock; block <= toBlock; block += this.BACKFILL_CHUNK_SIZE) {
        const endBlock = Math.min(block + this.BACKFILL_CHUNK_SIZE - 1, toBlock);

        try {
          for (let b = block; b <= endBlock; b++) {
            const blockData = await this.rateLimiter.executeWithLimit(async () => {
              return await this.alchemy.core.getBlockWithTransactions(b);
            });

            if (blockData && blockData.transactions) {
              for (const tx of blockData.transactions) {
                const valueStr = tx.value ? tx.value.toString() : '0x0';
                if (tx.value && valueStr !== '0x0' && tx.to) {
                  const timestamp = blockData.timestamp || Math.floor(Date.now() / 1000);

                  // Check duplicate
                  const isDup = await this.deduplicator.isDuplicate(
                    'native',
                    this.networkConfig.chainId,
                    tx.hash
                  );

                  if (!isDup) {
                    try {
                      await this.cache.storeNativeTransfer(
                        this.networkConfig.chainId,
                        tx.hash,
                        tx.from,
                        tx.to,
                        tx.value.toString(),
                        b,
                        timestamp
                      );

                      await this.deduplicator.markAsProcessed(
                        'native',
                        this.networkConfig.chainId,
                        tx.hash
                      );

                      totalTransfers++;
                    } catch (error) {
                      // Add to DLQ on error
                      await this.dlq.addToDLQ(
                        'native',
                        this.networkConfig.chainId,
                        {
                          chainId: this.networkConfig.chainId,
                          txHash: tx.hash,
                          from: tx.from,
                          to: tx.to,
                          value: tx.value.toString(),
                          blockNumber: b,
                          timestamp,
                        },
                        String(error)
                      );
                    }
                  }
                }
              }
            }
          }

          console.log(
            `[${this.networkConfig.name}] Backfill progress: blocks ${block}-${endBlock} (${totalTransfers} transfers so far)`
          );

          await new Promise((resolve) => setTimeout(resolve, 1000));
        } catch (error) {
          console.error(
            `[${this.networkConfig.name}] Error backfilling native blocks ${block}-${endBlock}:`,
            error
          );
          this.monitor.recordError(this.networkConfig.chainId);
        }
      }

      console.log(
        `[${this.networkConfig.name}] ✅ Native backfill complete: ${totalTransfers} transfers cached`
      );
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error during native backfill:`, error);
      this.monitor.recordError(this.networkConfig.chainId);
    }
  }

  private handleDisconnection(): void {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      console.error(
        `[${this.networkConfig.name}] ❌ Max reconnection attempts reached (native).`
      );
      return;
    }

    this.reconnectAttempts++;
    this.monitor.recordReconnection(this.networkConfig.chainId);

    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);

    console.log(
      `[${this.networkConfig.name}] Reconnecting (native) in ${delay / 1000}s...`
    );

    setTimeout(async () => {
      try {
        await this.setupWebSocketListener();
        this.reconnectAttempts = 0;
        console.log(`[${this.networkConfig.name}] ✅ Reconnected (native)`);
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Reconnection failed (native):`, error);
        this.handleDisconnection();
      }
    }, delay);
  }

  private setupConnectionMonitoring(): void {
    setInterval(async () => {
      try {
        if (!this.isShuttingDown) {
          const blockNumber = await this.alchemy.core.getBlockNumber();
          if (blockNumber < this.lastProcessedBlock) {
            console.warn(
              `[${this.networkConfig.name}] ⚠️  Block number went backwards (native).`
            );
          }
        }
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Connection check failed (native):`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        if (!this.isShuttingDown) {
          this.handleDisconnection();
        }
      }
    }, 30000);
  }

  private setupPeriodicSync(): void {
    const checkpointKey = `${this.networkConfig.chainId}_native`;

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
                `[${this.networkConfig.name}] Syncing ${gap} native blocks (${this.lastProcessedBlock + 1} to ${currentBlock})...`
              );

              try {
                await this.backfillBlocks(this.lastProcessedBlock + 1, currentBlock);
                this.lastProcessedBlock = currentBlock;
                await this.checkpoint.saveCheckpoint(parseInt(checkpointKey), currentBlock);
              } finally {
                this.isBackfilling = false; // Release lock
              }
            }
          }
        }
      } catch (error) {
        console.error(`[${this.networkConfig.name}] Periodic sync failed (native):`, error);
        this.monitor.recordError(this.networkConfig.chainId);
        this.isBackfilling = false; // Release lock on error
      }
    }, 15000); // Check every 15 seconds
  }

  stop(): void {
    this.isShuttingDown = true;
    this.alchemy.ws.removeAllListeners();
    console.log(`[${this.networkConfig.name}] Smart Reliable Native Listener stopped`);
  }
}
