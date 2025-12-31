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

The bug had **TWO issues** that combined to create the infinite loop:

**Issue 1: Missing Checkpoint Update**
The block listener's backfill didn't update `lastProcessedBlock` after completing:
- Block listener detects missed blocks and triggers backfill
- Backfill processes blocks 24133445-24133469
- `lastProcessedBlock` remains at 24133444 (never updated!)
- Next block arrives (24133470)
- Block listener thinks blocks 24133445-24133469 are STILL missed
- **Infinite loop**: Same blocks backfilled repeatedly

**Issue 2: Concurrent Backfills**
Two sources could trigger backfills simultaneously:
1. **Periodic Sync**: `setupPeriodicSync()` runs every 15 seconds
2. **Block Listener**: `alchemy.ws.on('block')` detects missed blocks in real-time

Without a lock, both would run concurrently on busy networks where backfills take >15 seconds

## Solution

Applied **TWO fixes** to solve both issues:

### Fix 1: Update Checkpoint After Block Listener Backfill
Added `lastProcessedBlock` update and checkpoint save after backfill completes in the block listener.

### Fix 2: Add Backfill Lock
Added a **backfill lock** (`isBackfilling` flag) to prevent concurrent backfills from both sources.

### Changes Made

**File**: `src/listeners/smartReliableErc20Listener.ts`
- Added `private isBackfilling = false;` flag
- Modified `setupPeriodicSync()` to check the lock before starting backfill
- Modified `setupWebSocketListener()` block handler to:
  - Check the lock before starting backfill
  - **Update `lastProcessedBlock` after backfill completes**
  - **Save checkpoint after backfill completes**
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
        // CRITICAL: Update lastProcessedBlock to prevent infinite loop
        this.lastProcessedBlock = blockNumber - 1;
        await this.checkpoint.saveCheckpoint(this.networkConfig.chainId, blockNumber - 1);
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
