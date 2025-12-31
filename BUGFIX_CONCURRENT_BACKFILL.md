# Bug Fix: Concurrent Backfill Issue

## Problem

The Ethereum listener (and potentially other networks) was stuck in a loop, repeatedly backfilling the same blocks.

### Symptoms

```
[Ethereum] Backfill chunk 24132685-24132694: found 3986 transfers
[Ethereum] Backfill chunk 24132695-24132704: found 5953 transfers
[Ethereum] Syncing 34 blocks (24132685 to 24132718)...
[Ethereum] Detected 34 missed block(s). Backfilling...
[Ethereum] Backfill chunk 24132685-24132694: found 3986 transfers  # REPEATING
[Ethereum] Backfill chunk 24132695-24132704: found 5953 transfers  # REPEATING
```

### Root Cause

There were **TWO sources** triggering backfills concurrently:

1. **Periodic Sync**: `setupPeriodicSync()` runs every 15 seconds
2. **Block Listener**: `alchemy.ws.on('block')` detects missed blocks in real-time

Both trigger backfills independently, and on busy networks like Ethereum:
- Backfilling takes longer than 15 seconds
- A new backfill would start before the previous one finished
- Multiple concurrent backfills processed the same blocks repeatedly
- This caused infinite loops and wasted API calls

## Solution

Added a **backfill lock** (`isBackfilling` flag) to prevent concurrent backfills from **BOTH sources**.

### Changes Made

**File**: `src/listeners/smartReliableErc20Listener.ts`
- Added `private isBackfilling = false;` flag
- Modified `setupPeriodicSync()` to check the lock before starting backfill
- Modified `setupWebSocketListener()` block handler to check the lock before starting backfill
- Lock is set before backfill starts, released when complete
- Lock is released even on error (using try/finally)

**File**: `src/listeners/smartReliableNativeListener.ts`
- Same changes as above for native transfer listener

### How It Works Now

**Periodic Sync**:
```typescript
setInterval(async () => {
  // Only proceed if not currently backfilling
  if (!this.isShuttingDown && !this.isBackfilling) {
    const currentBlock = await this.alchemy.core.getBlockNumber();

    if (currentBlock > this.lastProcessedBlock + 1) {
      this.isBackfilling = true; // Lock

      try {
        await this.backfillBlocks(...);
        this.lastProcessedBlock = currentBlock;
        await this.checkpoint.saveCheckpoint(...);
      } finally {
        this.isBackfilling = false; // Always release
      }
    }
  }
}, 15000);
```

**Block Listener**:
```typescript
this.alchemy.ws.on('block', async (blockNumber: number) => {
  if (blockNumber > this.lastProcessedBlock + 1 && this.lastProcessedBlock > 0) {
    const missedBlocks = blockNumber - this.lastProcessedBlock - 1;

    if (missedBlocks <= this.MAX_BACKFILL_BLOCKS && !this.isBackfilling) {
      this.isBackfilling = true; // Lock

      try {
        await this.backfillBlocks(this.lastProcessedBlock + 1, blockNumber - 1);
      } finally {
        this.isBackfilling = false; // Always release
      }
    }
  }

  this.lastProcessedBlock = blockNumber;
});
```

## Impact

**Before Fix**:
- Multiple backfills running concurrently
- Same blocks processed repeatedly
- Wasted API calls and CPU
- Logs filled with duplicate entries

**After Fix**:
- Only one backfill at a time
- Blocks processed once
- Efficient API usage
- Clean, linear progress logs

## Testing

After deploying the fix:

1. Rebuild: `npm run build`
2. Reload PM2: `npm run pm2:reload`
3. Watch logs: `pm2 logs blockchain-listener --lines 100`
4. Verify: Each block chunk should appear only once in logs

## Deployment

To apply this fix to your production server:

```bash
# Pull latest code
git pull

# Rebuild
npm run build

# Reload with zero downtime
npm run pm2:reload
```

---

**Date**: 2025-12-31
**Status**: Fixed
**Affects**: All networks with periodic sync enabled
