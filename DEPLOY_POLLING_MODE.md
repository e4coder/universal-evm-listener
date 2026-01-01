# Deploy: Switch to Polling Mode

## What Changed

✅ **Replaced WebSocket with Polling** using `getLogs`
✅ **Removed Native Transfer Tracking** (no event logs available)
✅ **Added Reorg Protection** (10-block lookback)
✅ **Simplified Architecture** (no WebSocket complexity)

## Quick Deploy

```bash
cd ~/universal_listener  # Or /home/sgxuser/universal-evm-listener

# Build
npm run build

# Restart
pm2 restart blockchain-listener

# Monitor
pm2 logs blockchain-listener --lines 50
```

## Expected Output

You should see:

```
🚀 Starting Universal Blockchain Listener (Polling Mode)...
📡 Monitoring 1 network(s) - ERC20 only
ℹ️  Native transfer tracking disabled (no event logs available)
✅ Redis connected
⏱️  Cache TTL: 1 hour(s)
🔄 Starting Dead Letter Queue auto-processing...
🏥 Starting health monitoring...
[Arbitrum One] Starting Polling ERC20 Listener...
[Arbitrum One] Found checkpoint at block 416557036 (current: 416557100)
[Arbitrum One] ✅ Polling ERC20 Listener active (poll every 2000ms)

✅ All listeners initialized
📊 Features: Polling-based, Checkpointing, Deduplication, DLQ, Reorg handling
🎯 Mode: getLogs with 10-block reorg safety, 3-block confirmation
🔁 Restarts: Auto-resume from last checkpoint

[Arbitrum One] Polling blocks 416557027 to 416557097 (current: 416557100)
[Arbitrum One] Found 234 Transfer events in blocks 416557027-416557097
[Arbitrum One] Polling blocks 416557098 to 416557195 (current: 416557198)
[Arbitrum One] Found 187 Transfer events in blocks 416557098-416557195
```

## Key Differences from WebSocket Mode

### Before (WebSocket)
- ❌ Constant WebSocket connection
- ❌ Heartbeat monitoring every 30s
- ❌ Reconnection logic with backoff
- ❌ "WebSocket dead" errors
- ❌ Unhandled close events
- ❌ Native transfer backfill loops
- ❌ "TOO MANY missed blocks" alerts

### After (Polling)
- ✅ Simple REST API calls every 2 seconds
- ✅ No connection management
- ✅ No reconnection needed
- ✅ Clean, predictable logs
- ✅ Automatic reorg handling
- ✅ No native transfer overhead
- ✅ Efficient filtered queries

## Performance

**Arbitrum (fast chain):**
- Poll every 2 seconds
- Query 10-15 blocks per poll
- Process ~100-300 Transfer events per poll
- ~2 API calls per second (sustainable)

**No more:**
- ❌ 240 block events per minute
- ❌ Overwhelming WebSocket stream
- ❌ Continuous backfill loops
- ❌ High API rate usage

## Native Transfers - Important Note

**Native transfers (ETH, MATIC, BNB, etc.) are NO LONGER tracked.**

**Why?**
Native transfers don't emit event logs. To track them, we'd need to fetch full blocks with all transactions, which is:
- Too slow (can't keep up with fast chains)
- Too expensive (rate limits)
- Not filterable (no getLogs support)

**Alternatives if you need native transfers:**
1. Use Alchemy's Transfer API (paid tier)
2. Run archive node with trace calls
3. Use a blockchain indexer (The Graph, etc.)

**For now: ERC20 tracking only is the most practical approach.**

## Verification

### Check Polling Activity
```bash
pm2 logs blockchain-listener | grep -i "polling"
```

### Check Events Stored
```bash
curl http://localhost:5459/erc20/address/42161/0x912CE59144191C1204E64559FE8253a0e49E6548
```

### Check Redis
```bash
redis-cli
> KEYS transfer:erc20:42161:*
> ZCARD idx:erc20:from:42161:0x912CE59144191C1204E64559FE8253a0e49E6548
```

## Rollback (if needed)

If you need to rollback to WebSocket mode:

```bash
# Revert code changes
git checkout HEAD~1 src/index.ts src/listeners/

# Rebuild
npm run build

# Restart
pm2 restart blockchain-listener
```

But polling mode is recommended - it's more reliable!

## API Changes

**No API changes** - all endpoints work the same:

```bash
# ERC20 endpoints still work
GET /erc20/from/:chainId/:address
GET /erc20/to/:chainId/:address
GET /erc20/address/:chainId/:address

# Native endpoints still exist but will return empty/old data
GET /native/from/:chainId/:address  # No new data
GET /native/to/:chainId/:address    # No new data
```

## Monitoring

Watch for these log patterns:

**Good:**
```
[Arbitrum One] Polling blocks X to Y (current: Z)
[Arbitrum One] Found N Transfer events in blocks X-Y
```

**Concerning:**
```
[Arbitrum One] Polling error: ...  # Should be rare
```

If polling errors occur, check:
1. Alchemy API key valid
2. Redis connection healthy
3. Network connectivity

## Benefits

1. **No more WebSocket issues** - no connection deaths, no reconnections
2. **No more backfill loops** - polling keeps up easily
3. **Better reorg handling** - automatic with block range queries
4. **Simpler code** - easier to maintain and debug
5. **More efficient** - filtered queries, less API usage

---

**Deploy with confidence - polling mode is the superior architecture!**
