import { Alchemy, AlchemySubscription } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';
import { NetworkConfig } from '../config/networks';

// ERC20 Transfer event signature
const ERC20_TRANSFER_EVENT = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';

export class ERC20Listener {
  private alchemy: Alchemy;
  private cache: RedisCache;
  private networkConfig: NetworkConfig;

  constructor(alchemy: Alchemy, cache: RedisCache, networkConfig: NetworkConfig) {
    this.alchemy = alchemy;
    this.cache = cache;
    this.networkConfig = networkConfig;
  }

  async start(): Promise<void> {
    console.log(`[${this.networkConfig.name}] Starting ERC20 Transfer listener...`);

    // Subscribe to all ERC20 Transfer events
    this.alchemy.ws.on(
      {
        method: AlchemySubscription.PENDING_TRANSACTIONS,
        // This will listen to all pending transactions, we'll filter for Transfer events
      },
      async (tx) => {
        // Note: For production, you'd want to use a more targeted subscription
        // This is a simplified version
      }
    );

    // Better approach: Listen to mined transactions and filter for Transfer events
    this.alchemy.ws.on(
      {
        method: AlchemySubscription.MINED_TRANSACTIONS,
        includeRemoved: false,
        hashesOnly: false,
      },
      async (tx: any) => {
        try {
          // Get full transaction receipt to check for logs
          if (tx.hash) {
            const receipt = await this.alchemy.core.getTransactionReceipt(tx.hash);

            if (receipt && receipt.logs) {
              for (const log of receipt.logs) {
                // Check if this is an ERC20 Transfer event
                if (log.topics[0] === ERC20_TRANSFER_EVENT && log.topics.length === 3) {
                  await this.handleTransferEvent(log, receipt.blockNumber);
                }
              }
            }
          }
        } catch (error) {
          console.error(`[${this.networkConfig.name}] Error processing transaction:`, error);
        }
      }
    );

    console.log(`[${this.networkConfig.name}] ERC20 Transfer listener started`);
  }

  private async handleTransferEvent(log: any, blockNumber: number): Promise<void> {
    try {
      // Decode Transfer event
      // topics[0] = event signature
      // topics[1] = from address (indexed)
      // topics[2] = to address (indexed)
      // data = value (not indexed)

      const from = '0x' + log.topics[1].slice(26); // Remove padding
      const to = '0x' + log.topics[2].slice(26); // Remove padding
      const value = log.data;

      // Get block to extract timestamp
      const block = await this.alchemy.core.getBlock(blockNumber);
      const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

      await this.cache.storeERC20Transfer(
        this.networkConfig.chainId,
        log.transactionHash,
        log.address, // Token contract address
        from,
        to,
        value,
        blockNumber,
        timestamp
      );

      console.log(
        `[${this.networkConfig.name}] ERC20 Transfer cached: ${from} -> ${to} (Token: ${log.address})`
      );
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error handling Transfer event:`, error);
    }
  }

  stop(): void {
    this.alchemy.ws.removeAllListeners();
    console.log(`[${this.networkConfig.name}] ERC20 Transfer listener stopped`);
  }
}
