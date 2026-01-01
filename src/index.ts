import 'dotenv/config';
import { Alchemy, AlchemySettings } from 'alchemy-sdk';
import { RedisCache } from './cache/redis';
import { SUPPORTED_NETWORKS } from './config/networks';
import { PollingERC20Listener } from './listeners/pollingErc20Listener';
import { QueryService } from './services/queryService';
import { CheckpointManager } from './persistence/checkpoint';
import { EventDeduplicator } from './utils/deduplication';
import { DeadLetterQueue } from './queue/deadLetterQueue';
import { EventMonitor } from './monitoring/eventMonitor';
import { RateLimiter } from './utils/rateLimiter';

class UniversalBlockchainListener {
  private cache: RedisCache;
  private queryService: QueryService;
  private listeners: PollingERC20Listener[] = [];

  // Reliability utilities
  private checkpoint: CheckpointManager;
  private deduplicator: EventDeduplicator;
  private dlq: DeadLetterQueue;
  private monitor: EventMonitor;
  private rateLimiter: RateLimiter;

  constructor() {
    this.cache = new RedisCache();
    this.queryService = new QueryService(this.cache);

    // Initialize reliability utilities
    this.checkpoint = new CheckpointManager(this.cache);
    this.deduplicator = new EventDeduplicator(this.cache);
    this.dlq = new DeadLetterQueue(this.cache);
    this.monitor = new EventMonitor();
    this.rateLimiter = new RateLimiter(200, 30); // 200 tokens, 30/sec refill (for 13 networks)
  }

  async start(): Promise<void> {
    console.log('🚀 Starting Universal Blockchain Listener (Polling Mode)...');
    console.log(`📡 Monitoring ${SUPPORTED_NETWORKS.length} network(s) - ERC20 only`);
    console.log('ℹ️  Native transfer tracking disabled (no event logs available)');

    // Connect to Redis
    await this.cache.connect();
    const cacheTTL = process.env.CACHE_TTL_HOURS || '1';
    console.log('✅ Redis connected');
    console.log(`⏱️  Cache TTL: ${cacheTTL} hour(s)`);

    // Start reliability services
    console.log('🔄 Starting Dead Letter Queue auto-processing...');
    this.dlq.startAutoProcessing();

    console.log('🏥 Starting health monitoring...');
    this.monitor.startHealthChecks();

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

        // Create polling ERC20 listener
        const erc20Listener = new PollingERC20Listener(
          alchemy,
          this.cache,
          networkConfig,
          this.checkpoint,
          this.deduplicator,
          this.dlq,
          this.monitor,
          this.rateLimiter
        );

        // Start listener
        await erc20Listener.start();

        this.listeners.push(erc20Listener);

        console.log(`✅ [${networkConfig.name}] Polling ERC20 Listener started`);
      } catch (error) {
        console.error(`❌ [${networkConfig.name}] Failed to start listener:`, error);
      }
    }

    console.log('\n✅ All listeners initialized');
    console.log('📊 Features: Polling-based, Checkpointing, Deduplication, DLQ, Reorg handling');
    console.log('🎯 Mode: getLogs with 10-block reorg safety, 3-block confirmation');
    console.log('🔁 Restarts: Auto-resume from last checkpoint\n');

    // Keep the process running
    this.setupGracefulShutdown();
  }

  private setupGracefulShutdown(): void {
    const shutdown = async () => {
      console.log('\n\n⏸️  Shutting down gracefully...');

      // Stop all listeners
      for (const listener of this.listeners) {
        listener.stop();
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
