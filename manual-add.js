// Manual script to add a specific transaction to cache
const redis = require('redis');

async function addTransaction() {
  const client = redis.createClient({
    url: process.env.REDIS_URL || 'redis://localhost:6379'
  });

  await client.connect();

  const chainId = 42161; // Arbitrum
  const txHash = '0xdd11c2c66628c22c1dd7b6e7f98679fa817e5eb9783013a17c4c1524f1e010ce';
  const token = '0xf97f4df75117a78c1a5a0dbb814af92458539fb4'; // LINK
  const from = '0xc59f2b56677e54627a19814306d67b04f7f9169d';
  const to = '0x6e76502cf3a5caf3e7a2e3774c8b2b5ccce4ae99';
  const value = '0x0dc36c13e2aa0000'; // 0.9925 LINK in hex
  const blockNumber = 416420602;
  const timestamp = Math.floor(Date.now() / 1000);

  const transfer = {
    txHash,
    token,
    from: from.toLowerCase(),
    to: to.toLowerCase(),
    value,
    blockNumber,
    timestamp,
    chainId
  };

  const transferKey = `transfer:erc20:${chainId}:${txHash}:${token}:${from}:${to}`;
  const TTL = 3600; // 1 hour

  await client.setEx(transferKey, TTL, JSON.stringify(transfer));

  // Index by 'from'
  await client.zAdd(`idx:erc20:from:${chainId}:${from.toLowerCase()}`, {
    score: timestamp,
    value: transferKey
  });
  await client.expire(`idx:erc20:from:${chainId}:${from.toLowerCase()}`, TTL);

  // Index by 'to'
  await client.zAdd(`idx:erc20:to:${chainId}:${to.toLowerCase()}`, {
    score: timestamp,
    value: transferKey
  });
  await client.expire(`idx:erc20:to:${chainId}:${to.toLowerCase()}`, TTL);

  // Index by both
  await client.zAdd(`idx:erc20:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`, {
    score: timestamp,
    value: transferKey
  });
  await client.expire(`idx:erc20:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`, TTL);

  console.log('✅ Transaction added to cache!');
  console.log(`   Chain: Arbitrum (${chainId})`);
  console.log(`   Token: LINK (${token})`);
  console.log(`   From: ${from}`);
  console.log(`   To: ${to}`);
  console.log(`   Value: 0.9925 LINK`);
  console.log(`   Block: ${blockNumber}`);

  await client.disconnect();
}

addTransaction().catch(console.error);
