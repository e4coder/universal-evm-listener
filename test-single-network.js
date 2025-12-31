// Temporary test script: Monitor only Arbitrum to avoid rate limits
require('dotenv/config');
const { Alchemy } = require('alchemy-sdk');
const { RedisCache } = require('./dist/cache/redis');
const { SmartReliableERC20Listener } = require('./dist/listeners/smartReliableErc20Listener');
const { SmartReliableNativeListener } = require('./dist/listeners/smartReliableNativeListener');
const { CheckpointManager } = require('./dist/persistence/checkpoint');
const { EventDeduplicator } = require('./dist/utils/deduplication');
const { DeadLetterQueue } = require('./dist/queue/deadLetterQueue');
const { EventMonitor } = require('./dist/monitoring/eventMonitor');
const { RateLimiter } = require('./dist/utils/rateLimiter');

const ARBITRUM_CONFIG = {
  name: 'Arbitrum One',
  chainId: 42161,
  alchemyNetwork: 'arb-mainnet',
  nativeSymbol: 'ETH'
};

async function start() {
  console.log('🚀 Starting Arbitrum-Only Listener (Test Mode)...');

  const cache = new RedisCache();
  await cache.connect();
  console.log('✅ Redis connected');

  const checkpoint = new CheckpointManager(cache);
  const deduplicator = new EventDeduplicator(cache);
  const dlq = new DeadLetterQueue(cache);
  const monitor = new EventMonitor();
  const rateLimiter = new RateLimiter(50, 5); // More conservative: 50 tokens, 5/sec

  dlq.startAutoProcessing();
  monitor.startHealthChecks();

  const alchemy = new Alchemy({
    apiKey: process.env.ALCHEMY_API_KEY,
    network: ARBITRUM_CONFIG.alchemyNetwork
  });

  const erc20Listener = new SmartReliableERC20Listener(
    alchemy, cache, ARBITRUM_CONFIG, checkpoint, deduplicator, dlq, monitor, rateLimiter
  );

  const nativeListener = new SmartReliableNativeListener(
    alchemy, cache, ARBITRUM_CONFIG, checkpoint, deduplicator, dlq, monitor, rateLimiter
  );

  await erc20Listener.start();
  await nativeListener.start();

  console.log('✅ Arbitrum listeners active!');
  console.log('💡 Send test transfers to: 0x6E76502cf3a5CAF3e7A2E3774c8B2B5cCCe4aE99');
  console.log('📊 Query: curl http://localhost:3000/all/42161/0x6E76502cf3a5CAF3e7A2E3774c8B2B5cCCe4aE99');

  // Graceful shutdown
  process.on('SIGINT', async () => {
    console.log('\n⏸️  Shutting down...');
    erc20Listener.stop();
    nativeListener.stop();
    await cache.disconnect();
    console.log('✅ Shutdown complete');
    process.exit(0);
  });
}

start().catch(console.error);
