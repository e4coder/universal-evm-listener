# Log Management Guide

## Overview

PM2 generates logs for both the blockchain listener and API server. Without proper management, these logs can grow indefinitely and fill up disk space.

## Automatic Log Rotation (Configured)

The deployment script automatically configures **pm2-logrotate** with these settings:

- **Max Size**: 10MB per log file
- **Retention**: 7 rotated files (about 1 week of logs)
- **Compression**: Enabled (saves disk space)
- **Rotation Time**: Daily at midnight

### What This Means

With these settings:
- Each log file grows to max 10MB before rotation
- 7 old rotated files are kept (compressed)
- Total disk usage: ~80MB per app (10MB current + 7x10MB compressed)
- **Total for both apps: ~160MB maximum**

## Log Locations

```bash
logs/
├── listener-error.log      # Current listener errors
├── listener-out.log        # Current listener output
├── api-error.log          # Current API errors
├── api-out.log            # Current API output
├── listener-error__*.log.gz  # Rotated/compressed
├── listener-out__*.log.gz    # Rotated/compressed
├── api-error__*.log.gz       # Rotated/compressed
└── api-out__*.log.gz         # Rotated/compressed
```

## Manual Log Management

### View Current Logs

```bash
# Live tail all logs
pm2 logs

# View specific app
pm2 logs blockchain-listener
pm2 logs blockchain-api

# View last 100 lines
pm2 logs --lines 100

# View only errors
pm2 logs --err
```

### Clear Logs Manually

```bash
# Clear all logs
pm2 flush

# Or delete log files
rm -rf logs/*.log
pm2 restart all  # Recreate log files
```

### Archive Old Logs

```bash
# Create archive
tar -czf logs-archive-$(date +%Y%m%d).tar.gz logs/*.log.gz

# Move to archive directory
mkdir -p ~/log-archives
mv logs-archive-*.tar.gz ~/log-archives/

# Clean up old compressed logs
rm logs/*.log.gz
```

## Monitoring Disk Usage

### Check Log Size

```bash
# Total size of logs directory
du -sh logs/

# Size of each log file
ls -lh logs/

# Find large files
find logs/ -size +5M -ls
```

### Automated Monitoring Script

Create `check-logs.sh`:

```bash
#!/bin/bash
MAX_SIZE_MB=200

CURRENT_SIZE=$(du -sm logs/ | cut -f1)

if [ $CURRENT_SIZE -gt $MAX_SIZE_MB ]; then
    echo "⚠️  WARNING: Logs directory is ${CURRENT_SIZE}MB (max: ${MAX_SIZE_MB}MB)"
    echo "Consider archiving or clearing old logs"
else
    echo "✅ Logs size OK: ${CURRENT_SIZE}MB / ${MAX_SIZE_MB}MB"
fi
```

## pm2-logrotate Configuration

### Check Current Settings

```bash
pm2 conf pm2-logrotate
```

### Update Settings

```bash
# Change max size to 20MB
pm2 set pm2-logrotate:max_size 20M

# Keep 14 files (2 weeks)
pm2 set pm2-logrotate:retain 14

# Disable compression
pm2 set pm2-logrotate:compress false

# Check immediately for rotation
pm2 set pm2-logrotate:workerInterval 1
```

### Verify Log Rotation is Working

```bash
# Check if pm2-logrotate is installed
pm2 list | grep logrotate

# View pm2-logrotate logs
pm2 logs pm2-logrotate

# Trigger manual rotation
pm2 trigger pm2-logrotate rotate
```

## Troubleshooting

### Logs Growing Too Fast

If logs are growing faster than expected:

1. **Check for errors causing repeated messages**
   ```bash
   pm2 logs --err --lines 100
   ```

2. **Reduce log verbosity** (if you added debug logging)
   - Edit listener/API code to reduce console.log statements

3. **Increase rotation frequency**
   ```bash
   pm2 set pm2-logrotate:max_size 5M  # Rotate more frequently
   ```

### Disk Space Running Out

Emergency cleanup:

```bash
# Stop applications
pm2 stop all

# Archive current logs
tar -czf emergency-backup-$(date +%Y%m%d-%H%M%S).tar.gz logs/

# Clear all logs
rm -rf logs/*

# Restart applications
pm2 restart all
```

### pm2-logrotate Not Working

```bash
# Reinstall pm2-logrotate
pm2 uninstall pm2-logrotate
pm2 install pm2-logrotate

# Reconfigure
pm2 set pm2-logrotate:max_size 10M
pm2 set pm2-logrotate:retain 7
pm2 set pm2-logrotate:compress true
```

## Best Practices

1. **Monitor Regularly**
   ```bash
   # Add to daily checks
   du -sh logs/
   ```

2. **Archive Monthly**
   ```bash
   # Archive and clear old compressed logs
   tar -czf ~/archives/logs-$(date +%Y-%m).tar.gz logs/*.gz
   rm logs/*.log.gz
   ```

3. **Set Up Alerts** (optional)
   - Use monitoring tools (Datadog, New Relic, etc.)
   - Set up disk space alerts

4. **Review Logs Weekly**
   ```bash
   # Check for recurring errors
   pm2 logs --err --lines 1000 | grep -i error | sort | uniq -c | sort -rn
   ```

5. **Clean Up After Issues**
   ```bash
   # After resolving an issue, clear error logs
   > logs/listener-error.log
   > logs/api-error.log
   ```

## Cron Job for Automatic Archival

Add to crontab (`crontab -e`):

```bash
# Archive logs monthly (1st of month at 2 AM)
0 2 1 * * cd /path/to/universal_listener && tar -czf ~/log-archives/logs-$(date +\%Y-\%m).tar.gz logs/*.log.gz && rm logs/*.log.gz

# Check disk usage daily (8 AM)
0 8 * * * cd /path/to/universal_listener && du -sh logs/ >> ~/log-size-history.txt
```

## Expected Log Volume

For reference, with 13 networks running:

- **Normal Operation**: 2-5MB per day per log file
- **During Backfill**: 10-20MB per day per log file
- **With Errors**: Could grow faster

With rotation at 10MB and 7 files retained:
- **Worst Case**: ~160MB total disk usage
- **Typical**: 50-80MB total disk usage

---

**Summary**: Logs are automatically managed and will NOT overwhelm your disk if pm2-logrotate is configured (which the deploy script does automatically).

**Last Updated**: 2025-12-31
