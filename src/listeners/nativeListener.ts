import { Alchemy, AlchemySubscription } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';
import { NetworkConfig } from '../config/networks';

export class NativeTransferListener {
  private alchemy: Alchemy;
  private cache: RedisCache;
  private networkConfig: NetworkConfig;

  constructor(alchemy: Alchemy, cache: RedisCache, networkConfig: NetworkConfig) {
    this.alchemy = alchemy;
    this.cache = cache;
    this.networkConfig = networkConfig;
  }

  async start(): Promise<void> {
    console.log(`[${this.networkConfig.name}] Starting Native Transfer listener...`);

    // Listen to mined transactions and filter for native transfers
    this.alchemy.ws.on(
      {
        method: AlchemySubscription.MINED_TRANSACTIONS,
        includeRemoved: false,
        hashesOnly: false,
      },
      async (tx: any) => {
        try {
          // Check if transaction has value (native token transfer)
          if (tx.transaction && tx.transaction.value && tx.transaction.value !== '0x0') {
            await this.handleNativeTransfer(tx.transaction);
          }
        } catch (error) {
          console.error(`[${this.networkConfig.name}] Error processing native transfer:`, error);
        }
      }
    );

    console.log(`[${this.networkConfig.name}] Native Transfer listener started`);
  }

  private async handleNativeTransfer(tx: any): Promise<void> {
    try {
      const from = tx.from;
      const to = tx.to;
      const value = tx.value;
      const txHash = tx.hash;

      // Skip if 'to' is null (contract creation)
      if (!to) return;

      // Get transaction receipt to get block number
      const receipt = await this.alchemy.core.getTransactionReceipt(txHash);
      if (!receipt) return;

      const blockNumber = receipt.blockNumber;

      // Get block to extract timestamp
      const block = await this.alchemy.core.getBlock(blockNumber);
      const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

      await this.cache.storeNativeTransfer(
        this.networkConfig.chainId,
        txHash,
        from,
        to,
        value,
        blockNumber,
        timestamp
      );

      console.log(
        `[${this.networkConfig.name}] Native Transfer cached: ${from} -> ${to} (${this.networkConfig.nativeSymbol})`
      );
    } catch (error) {
      console.error(`[${this.networkConfig.name}] Error handling native transfer:`, error);
    }
  }

  stop(): void {
    this.alchemy.ws.removeAllListeners();
    console.log(`[${this.networkConfig.name}] Native Transfer listener stopped`);
  }
}
