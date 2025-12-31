/**
 * Admin endpoint for manual backfilling
 * Use this to backfill specific block ranges when needed
 */

import { Alchemy } from 'alchemy-sdk';
import { RedisCache } from '../cache/redis';

const ERC20_TRANSFER_EVENT = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';

export async function backfillBlocks(
  alchemy: Alchemy,
  cache: RedisCache,
  chainId: number,
  fromBlock: number,
  toBlock: number
): Promise<{ processed: number; errors: number }> {
  console.log(`[Backfill] Starting backfill for chain ${chainId}: blocks ${fromBlock} to ${toBlock}`);

  let processed = 0;
  let errors = 0;

  // Process in chunks to avoid overwhelming API
  const CHUNK_SIZE = 1000;

  for (let start = fromBlock; start <= toBlock; start += CHUNK_SIZE) {
    const end = Math.min(start + CHUNK_SIZE - 1, toBlock);

    try {
      // Get ERC20 Transfer logs
      const logs = await alchemy.core.getLogs({
        fromBlock: start,
        toBlock: end,
        topics: [ERC20_TRANSFER_EVENT],
      });

      console.log(`[Backfill] Found ${logs.length} ERC20 transfers in blocks ${start}-${end}`);

      for (const log of logs) {
        try {
          if (log.topics.length === 3) {
            const from = '0x' + log.topics[1].slice(26);
            const to = '0x' + log.topics[2].slice(26);
            const value = log.data;
            const blockNumber = log.blockNumber || start;

            // Get block for timestamp
            const block = await alchemy.core.getBlock(blockNumber);
            const timestamp = block?.timestamp || Math.floor(Date.now() / 1000);

            await cache.storeERC20Transfer(
              chainId,
              log.transactionHash,
              log.address,
              from,
              to,
              value,
              blockNumber,
              timestamp
            );

            processed++;
          }
        } catch (error) {
          console.error(`[Backfill] Error processing log:`, error);
          errors++;
        }
      }

      // Rate limiting - wait between chunks
      await new Promise((resolve) => setTimeout(resolve, 200));
    } catch (error) {
      console.error(`[Backfill] Error getting logs for blocks ${start}-${end}:`, error);
      errors++;
    }
  }

  console.log(`[Backfill] Complete: ${processed} events processed, ${errors} errors`);
  return { processed, errors };
}

/**
 * Example usage in API server:
 *
 * POST /admin/backfill
 * Body: { chainId: 1, fromBlock: 19000000, toBlock: 19001000 }
 */
