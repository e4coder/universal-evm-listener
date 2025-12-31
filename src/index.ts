import 'dotenv/config';
import { Alchemy, AlchemySettings } from 'alchemy-sdk';
import { RedisCache } from './cache/redis';
import { SUPPORTED_NETWORKS } from './config/networks';
import { ERC20Listener } from './listeners/erc20Listener';
import { NativeTransferListener } from './listeners/nativeListener';
import { QueryService } from './services/queryService';

class UniversalBlockchainListener {
  private cache: RedisCache;
  private queryService: QueryService;
  private listeners: Array<{ erc20: ERC20Listener; native: NativeTransferListener }> = [];

  constructor() {
    this.cache = new RedisCache();
    this.queryService = new QueryService(this.cache);
  }

  async start(): Promise<void> {
    console.log('🚀 Starting Universal Blockchain Listener...');
    console.log(`📡 Monitoring ${SUPPORTED_NETWORKS.length} networks`);

    // Connect to Redis
    await this.cache.connect();
    const cacheTTL = process.env.CACHE_TTL_HOURS || '1';
    console.log('✅ Redis connected');
    console.log(`⏱️  Cache TTL: ${cacheTTL} hour(s)`);

    const apiKey = process.env.ALCHEMY_API_KEY;
    if (!apiKey) {
      throw new Error('ALCHEMY_API_KEY is not set in environment variables');
    }

    // Initialize listeners for each network
    for (const networkConfig of SUPPORTED_NETWORKS) {
      try {
        const settings: AlchemySettings = {
          apiKey: apiKey,
          network: networkConfig.alchemyNetwork,
        };

        const alchemy = new Alchemy(settings);

        // Create listeners
        const erc20Listener = new ERC20Listener(alchemy, this.cache, networkConfig);
        const nativeListener = new NativeTransferListener(alchemy, this.cache, networkConfig);

        // Start listeners
        await erc20Listener.start();
        await nativeListener.start();

        this.listeners.push({ erc20: erc20Listener, native: nativeListener });

        console.log(`✅ [${networkConfig.name}] Listeners started successfully`);
      } catch (error) {
        console.error(`❌ [${networkConfig.name}] Failed to start listeners:`, error);
      }
    }

    console.log('\n✅ All listeners initialized');
    console.log('📊 Listening for ERC20 and Native transfers on all networks...\n');

    // Keep the process running
    this.setupGracefulShutdown();
  }

  private setupGracefulShutdown(): void {
    const shutdown = async () => {
      console.log('\n\n⏸️  Shutting down gracefully...');

      // Stop all listeners
      for (const listener of this.listeners) {
        listener.erc20.stop();
        listener.native.stop();
      }

      // Disconnect from Redis
      await this.cache.disconnect();
      console.log('✅ Redis disconnected');

      console.log('👋 Shutdown complete');
      process.exit(0);
    };

    process.on('SIGINT', shutdown);
    process.on('SIGTERM', shutdown);
  }

  // Expose query service for external use
  getQueryService(): QueryService {
    return this.queryService;
  }
}

// Start the application
const app = new UniversalBlockchainListener();
app.start().catch((error) => {
  console.error('❌ Failed to start application:', error);
  process.exit(1);
});

// Export for use as a module
export { UniversalBlockchainListener, QueryService };
