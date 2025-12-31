# Deploy Latest Fixes - December 31, 2025

## What's Included

This deployment includes **TWO critical fixes** and **ONE major optimization**:

### 1. Bugfix: Infinite Backfill Loop (CRITICAL)
- **Issue**: Ethereum, OP Mainnet, and other networks stuck repeating the same blocks
- **Root Cause**: Block listener didn't update checkpoint after backfill completion
- **Fix**: Added checkpoint update after block listener backfills
- **Impact**: Eliminates infinite loops, stops wasting API calls
- **Details**: See [BUGFIX_CONCURRENT_BACKFILL.md](BUGFIX_CONCURRENT_BACKFILL.md)

### 2. Optimization: Native Transfer API Calls (MAJOR)
- **Issue**: 2 API calls per native transfer (extremely wasteful)
- **Root Cause**: Unnecessary `getTransactionReceipt` calls and no block caching
- **Fix**: Use `tx.blockNumber` directly + block cache
- **Impact**: 99.95% reduction in API calls on high-volume networks
- **Details**: See [OPTIMIZATION_API_CALLS.md](OPTIMIZATION_API_CALLS.md)

### 3. Enhancement: Log Rotation
- **Issue**: Logs could fill disk space
- **Fix**: Automatic PM2 log rotation (10MB max, 7 files, compressed)
- **Details**: See [LOG_MANAGEMENT.md](LOG_MANAGEMENT.md)

## Pre-Deployment Checklist

- [ ] Read [BUGFIX_CONCURRENT_BACKFILL.md](BUGFIX_CONCURRENT_BACKFILL.md)
- [ ] Read [OPTIMIZATION_API_CALLS.md](OPTIMIZATION_API_CALLS.md)
- [ ] Backup current Redis data (optional, checkpoints are safe)
- [ ] Ensure you have SSH access to production server
- [ ] Check current PM2 status: `pm2 status`

## Deployment Steps

### Step 1: Pull Latest Code

```bash
cd /home/ubuntu/universal_listener
git pull
```

**Expected output**: File changes for `smartReliableErc20Listener.ts` and `smartReliableNativeListener.ts`

### Step 2: Rebuild

```bash
npm run build
```

**Expected output**: TypeScript compilation with no errors

### Step 3: Reload PM2 (Zero Downtime)

```bash
npm run pm2:reload
```

**Expected output**:
```
✅ blockchain-listener reloaded
✅ blockchain-api reloaded
```

### Step 4: Verify Deployment

Watch the logs for 1-2 minutes:

```bash
pm2 logs blockchain-listener --lines 100
```

**What to look for**:

✅ **Good signs** (bug is fixed):
- Each block range appears only ONCE
- Clean, linear progression: "Backfilling blocks 24133500-24133525..."
- No repeating block numbers
- Networks initialize and start processing
- "✅ Backfill complete" messages

❌ **Bad signs** (something wrong):
- Same block ranges appearing multiple times
- "Detected X missed blocks" immediately after backfill completes for same range
- Errors or crashes

### Step 5: Monitor API Usage

After 5-10 minutes, check Alchemy dashboard for:
- **Reduced API call rate** (especially on Ethereum and OP Mainnet)
- **No rate limit warnings**
- **Smooth, consistent usage pattern**

## Expected Behavior After Deployment

### Ethereum
**Before**: Infinite loop processing blocks 24133445-24133470 repeatedly
**After**: Clean progression through blocks, each processed once

### OP Mainnet
**Before**: Massive concurrent backfills (35, 36, 37... up to 51 missed blocks)
**After**: Orderly backfills, no concurrency, linear progression

### All Networks (Native Transfers)
**Before**: 2 API calls per transfer (thousands per block on Ethereum)
**After**: ~1 API call per block (99.95% reduction)

## Rollback Plan (If Needed)

If you encounter critical issues after deployment:

```bash
# Stop the listeners
pm2 stop blockchain-listener

# Revert to previous commit
git log --oneline -5  # Find previous commit hash
git checkout <previous-commit-hash>

# Rebuild and restart
npm run build
pm2 restart blockchain-listener

# Check logs
pm2 logs blockchain-listener
```

## Post-Deployment Verification

Run these checks after 10 minutes of operation:

### 1. Check Process Status
```bash
pm2 status
```
All processes should be **online** with **0 restarts**.

### 2. Check Logs for Errors
```bash
pm2 logs blockchain-listener --lines 200 | grep -i error
```
Should show minimal/no errors.

### 3. Check Block Progress
```bash
pm2 logs blockchain-listener --lines 50 | grep "Backfill"
```
Each block range should appear only once, with increasing block numbers.

### 4. Check API Health
```bash
curl http://localhost:5459/networks
```
Should return JSON with all 13 networks.

### 5. Monitor Redis
```bash
redis-cli
> KEYS checkpoint:*
> KEYS transfer:*
```
Checkpoints should be updating to higher block numbers.

## Troubleshooting

### Issue: Infinite loop persists

**Check**: Did the code actually update?
```bash
grep -n "Update lastProcessedBlock to prevent re-processing" src/listeners/smartReliableErc20Listener.ts
```
Should show the comment on line ~124.

**Fix**: Ensure git pull worked, rebuild, and reload again.

### Issue: Process crashes immediately

**Check logs**:
```bash
pm2 logs blockchain-listener --err --lines 100
```

**Common causes**:
- Redis connection issue
- Alchemy API key invalid
- Missing environment variables

**Fix**: Check .env file and Redis status.

### Issue: High API usage continues

**Wait**: Block cache needs to warm up (2-3 minutes)
**Verify**: Check that optimization code is present:
```bash
grep -n "blockCache" src/listeners/smartReliableNativeListener.ts
```
Should show cache declaration and usage.

## Success Metrics

After 30 minutes of operation, you should see:

| Metric | Before | After |
|--------|--------|-------|
| Ethereum API calls/min | ~10,000 | ~50-100 |
| OP Mainnet API calls/min | ~5,000 | ~30-50 |
| Log spam (repeating blocks) | Severe | None |
| Block processing | Stuck/looping | Linear progression |
| PM2 restarts | Frequent | 0 |

## Additional Resources

- [BUGFIX_CONCURRENT_BACKFILL.md](BUGFIX_CONCURRENT_BACKFILL.md) - Detailed bug analysis
- [OPTIMIZATION_API_CALLS.md](OPTIMIZATION_API_CALLS.md) - API optimization details
- [LOG_MANAGEMENT.md](LOG_MANAGEMENT.md) - Log rotation setup
- [PM2_DEPLOYMENT.md](PM2_DEPLOYMENT.md) - PM2 operations guide

## Support

If you encounter issues not covered in this guide:

1. Check all related documentation files
2. Review PM2 logs: `pm2 logs blockchain-listener --lines 500`
3. Check Redis: `redis-cli ping`
4. Verify environment: `cat .env`

---

**Deployment Date**: 2025-12-31
**Critical Fixes**: Infinite backfill loop
**Optimizations**: Native transfer API calls (99.95% reduction)
**Backwards Compatible**: Yes
**Downtime Required**: No (zero-downtime reload)
