# API Call Optimization: Native Transfer Listener

## Problem

The native transfer listener was making **2 API calls per transfer**:
1. `getTransactionReceipt(txHash)` - to get block number
2. `getBlock(blockNumber)` - to get timestamp

On high-volume networks like Ethereum with thousands of native transfers per block, this was **extremely wasteful** and would quickly exhaust API rate limits.

### Before Optimization

```typescript
// For EVERY native transfer:
const receipt = await this.alchemy.core.getTransactionReceipt(txHash); // API call #1
const blockNumber = receipt.blockNumber;
const block = await this.alchemy.core.getBlock(blockNumber);           // API call #2
const timestamp = block?.timestamp;
```

**Cost per transfer**: 2 API calls
**Ethereum example**: 1,000 native transfers/block × 2 = **2,000 API calls per block**

## Solution

Applied **TWO optimizations**:

### Optimization 1: Eliminate getTransactionReceipt

The transaction object **already contains the block number** - no need to fetch the receipt!

```typescript
// Use blockNumber directly from tx object
const blockNumber = tx.blockNumber; // No API call needed!
```

**Savings**: Eliminates 1 API call per transfer (50% reduction)

### Optimization 2: Block Caching

Multiple transfers in the same block need the same timestamp. Cache blocks to avoid redundant `getBlock` calls.

```typescript
// Check cache first
const cachedBlock = this.blockCache.get(blockNumber);
if (cachedBlock) {
  timestamp = cachedBlock.timestamp; // Use cached value - 0 API calls!
} else {
  // Fetch and cache
  const block = await this.alchemy.core.getBlock(blockNumber);
  this.blockCache.set(blockNumber, { timestamp: block.timestamp });
}
```

**Cache size**: 100 blocks (LRU eviction)
**Savings**: 1 API call per block instead of 1 per transfer

## Impact

### Before Optimization
- **Per transfer**: 2 API calls
- **Per block** (1,000 transfers): 2,000 API calls
- **Per minute** (5 blocks): 10,000 API calls

### After Optimization
- **Per transfer**: ~0 API calls (cached)
- **Per block** (1,000 transfers): 1 API call (cache miss)
- **Per minute** (5 blocks): 5 API calls

**Total reduction**: **99.95% fewer API calls** on high-volume networks!

## Technical Details

### Changes Made

**File**: `src/listeners/smartReliableNativeListener.ts`

**Line 25**: Added block cache
```typescript
private blockCache: Map<number, { timestamp: number }> = new Map();
```

**Line 188-212**: Optimized handleNativeTransfer
- Removed `getTransactionReceipt` call
- Use `tx.blockNumber` directly
- Check cache before calling `getBlock`
- LRU cache eviction when size exceeds 100 blocks

### Why This Works

1. **Block numbers are in transactions**: The Alchemy SDK transaction object includes `blockNumber`
2. **Blocks are immutable**: Once a block is mined, its timestamp never changes - safe to cache
3. **Temporal locality**: Transfers arrive in bursts per block - high cache hit rate
4. **Memory efficient**: 100 blocks × ~50 bytes = ~5KB memory overhead

## Deployment

This optimization is **100% backwards compatible** - no changes needed to Redis, checkpoints, or data structures.

To deploy:

```bash
cd /home/ubuntu/universal_listener
git pull
npm run build
npm run pm2:reload
```

## Verification

After deployment, monitor the logs:

**Before**: Heavy API usage warnings on Ethereum/OP Mainnet
**After**: Smooth operation with minimal API calls

You can verify the optimization by checking:
1. Reduced rate limit warnings
2. Faster backfill completion
3. Lower API usage in Alchemy dashboard

---

**Date**: 2025-12-31
**Status**: Implemented
**Affects**: Native transfer listeners on all networks
**API Call Reduction**: ~99.95% on high-volume networks
