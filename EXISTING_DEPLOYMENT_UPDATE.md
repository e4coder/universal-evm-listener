# Update Existing Deployment

Since you already deployed with `./deploy.sh`, here's what to do to add log rotation to your existing deployment.

## Quick Setup (Recommended)

On your production server, run:

```bash
./setup-logrotate.sh
```

That's it! The script will:
- ✅ Install pm2-logrotate
- ✅ Configure rotation settings
- ✅ Save configuration

## Manual Setup (Alternative)

If you prefer to do it manually:

```bash
# 1. Install pm2-logrotate
pm2 install pm2-logrotate

# 2. Wait a few seconds for installation
sleep 3

# 3. Configure settings
pm2 set pm2-logrotate:max_size 10M
pm2 set pm2-logrotate:retain 7
pm2 set pm2-logrotate:compress true
pm2 set pm2-logrotate:dateFormat YYYY-MM-DD_HH-mm-ss

# 4. Save configuration
pm2 save
```

## Verify It's Working

```bash
# Check if pm2-logrotate is running
pm2 list | grep logrotate

# View configuration
pm2 conf pm2-logrotate

# View pm2-logrotate logs
pm2 logs pm2-logrotate --lines 50
```

## What This Does

- **Prevents log overflow**: Logs won't grow forever
- **Automatic rotation**: When a log hits 10MB, it rotates
- **Compression**: Old logs are compressed to save space
- **Retention**: Keeps 7 old files, deletes older ones
- **Total disk usage**: ~160MB maximum for all logs

## Current Logs

Your existing logs will remain untouched. Rotation will start applying to new log data.

```bash
# Check current log size
du -sh logs/

# View current logs
pm2 logs
```

## Future Deployments

The updated `deploy.sh` script now automatically sets up log rotation, so future deployments won't need this manual step.

## Questions?

- Check log size: `du -sh logs/`
- View logs: `pm2 logs`
- Check rotation: `pm2 logs pm2-logrotate`
- See full guide: `LOG_MANAGEMENT.md`

---

**Last Updated**: 2025-12-31
