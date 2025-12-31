import 'dotenv/config';
import { RedisCache } from '../src/cache/redis';
import { QueryService } from '../src/services/queryService';

/**
 * Example script demonstrating how to query cached blockchain transfer data
 */

async function main() {
  const cache = new RedisCache();
  await cache.connect();

  const queryService = new QueryService(cache);

  // Example address (replace with actual address)
  const exampleAddress = '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb';

  console.log('🔍 Querying blockchain transfer data...\n');

  // Query Ethereum (chainId: 1)
  console.log('=== Ethereum (Chain ID: 1) ===');

  // Get all ERC20 transfers for the address
  const ethErc20Transfers = await queryService.getERC20TransfersByAddress(1, exampleAddress);
  console.log(`ERC20 Transfers: ${ethErc20Transfers.length}`);

  // Get all native ETH transfers for the address
  const ethNativeTransfers = await queryService.getNativeTransfersByAddress(1, exampleAddress);
  console.log(`Native ETH Transfers: ${ethNativeTransfers.length}`);

  // Get all transfers (both ERC20 and native)
  const allEthTransfers = await queryService.getAllTransfersByAddress(1, exampleAddress);
  console.log(`Total Transfers: ${allEthTransfers.total}`);
  console.log(`  - ERC20: ${allEthTransfers.erc20.length}`);
  console.log(`  - Native: ${allEthTransfers.native.length}`);

  console.log('\n=== Polygon (Chain ID: 137) ===');

  // Query Polygon transfers
  const polygonTransfers = await queryService.getAllTransfersByAddress(137, exampleAddress);
  console.log(`Total Transfers: ${polygonTransfers.total}`);

  console.log('\n=== Query by specific direction ===');

  // Get only transfers FROM this address on Ethereum
  const sentTransfers = await queryService.getERC20TransfersByFrom(1, exampleAddress);
  console.log(`ERC20 Transfers sent FROM address: ${sentTransfers.length}`);

  // Get only transfers TO this address on Ethereum
  const receivedTransfers = await queryService.getERC20TransfersByTo(1, exampleAddress);
  console.log(`ERC20 Transfers sent TO address: ${receivedTransfers.length}`);

  // Get transfers between two specific addresses
  const anotherAddress = '0x1234567890123456789012345678901234567890';
  const betweenTransfers = await queryService.getERC20TransfersByBoth(
    1,
    exampleAddress,
    anotherAddress
  );
  console.log(`ERC20 Transfers between ${exampleAddress} and ${anotherAddress}: ${betweenTransfers.length}`);

  console.log('\n=== Sample Transfer Data ===');
  if (allEthTransfers.erc20.length > 0) {
    console.log('First ERC20 Transfer:');
    console.log(JSON.stringify(allEthTransfers.erc20[0], null, 2));
  }

  if (allEthTransfers.native.length > 0) {
    console.log('\nFirst Native Transfer:');
    console.log(JSON.stringify(allEthTransfers.native[0], null, 2));
  }

  await cache.disconnect();
  console.log('\n✅ Query complete');
}

main().catch((error) => {
  console.error('❌ Error:', error);
  process.exit(1);
});
