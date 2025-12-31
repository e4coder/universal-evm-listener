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

The `setupPeriodicSync()` function runs every 15 seconds and triggers backfills. However:

1. Backfilling takes longer than 15 seconds for busy networks like Ethereum
2. The periodic check would start a NEW backfill before the previous one finished
3. Multiple concurrent backfills were running, processing the same blocks repeatedly
4. This caused infinite loops and wasted API calls

## Solution

Added a **backfill lock** (`isBackfilling` flag) to prevent concurrent backfills.

### Changes Made

**File**: `src/listeners/smartReliableErc20Listener.ts`
- Added `private isBackfilling = false;` flag
- Modified `setupPeriodicSync()` to check the lock before starting backfill
- Lock is set before backfill starts, released when complete
- Lock is released even on error (using try/finally)

**File**: `src/listeners/smartReliableNativeListener.ts`
- Same changes as above for native transfer listener

### How It Works Now

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
