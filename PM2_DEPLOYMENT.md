# PM2 Deployment Guide

Complete guide for deploying the Universal Blockchain Listener with PM2.

## Prerequisites

```bash
# Install PM2 globally
npm install -g pm2

# Install PM2 log rotation (optional but recommended)
pm2 install pm2-logrotate
```

## Initial Deployment

### 1. Clone and Setup

```bash
# Clone your repository
git clone <your-repo-url>
cd universal_listener

# Install dependencies
npm install

# Configure environment
cp .env.example .env
nano .env  # Add your ALCHEMY_API_KEY
```

### 2. Build the Project

```bash
npm run build
```

### 3. Start Redis

```bash
# Start Redis with Docker Compose
docker compose up -d

# Verify Redis is running
docker ps | grep redis
```

### 4. Start Applications with PM2

```bash
# Start both listener and API server
pm2 start ecosystem.config.js

# Save PM2 process list
pm2 save

# Setup PM2 to start on system boot
pm2 startup
# Follow the instructions printed by the command above
```

## PM2 Commands

### Basic Operations

```bash
# View all running processes
pm2 list

# View logs (all apps)
pm2 logs

# View logs for specific app
pm2 logs blockchain-listener
pm2 logs blockchain-api

# Monitor in real-time
pm2 monit

# Stop applications
pm2 stop blockchain-listener
pm2 stop blockchain-api
pm2 stop all

# Restart applications
pm2 restart blockchain-listener
pm2 restart blockchain-api
pm2 restart all

# Reload (zero-downtime restart)
pm2 reload blockchain-listener
pm2 reload blockchain-api

# Delete from PM2
pm2 delete blockchain-listener
pm2 delete blockchain-api
pm2 delete all
```

### Monitoring

```bash
# Show detailed info
pm2 show blockchain-listener
pm2 show blockchain-api

# Monitor CPU and memory
pm2 monit

# View real-time logs
pm2 logs --lines 100

# Flush logs
pm2 flush
```

## Log Management

### Manual Log Rotation

```bash
# View current logs
ls -lh logs/

# Rotate logs manually
pm2 flush

# Archive old logs
tar -czf logs-archive-$(date +%Y%m%d).tar.gz logs/*.log
```

### Automated Log Rotation with pm2-logrotate

```bash
# Install pm2-logrotate
pm2 install pm2-logrotate

# Configure rotation (10MB max size, keep 7 files)
pm2 set pm2-logrotate:max_size 10M
pm2 set pm2-logrotate:retain 7
pm2 set pm2-logrotate:compress true
pm2 set pm2-logrotate:dateFormat YYYY-MM-DD_HH-mm-ss
```

## Updating the Application

### Zero-Downtime Deployment

```bash
# Pull latest changes
git pull origin main

# Install new dependencies (if any)
npm install

# Rebuild
npm run build

# Reload applications (zero-downtime)
pm2 reload ecosystem.config.js

# Or reload individually
pm2 reload blockchain-listener
pm2 reload blockchain-api
```

### Full Restart Deployment

```bash
# Pull latest changes
git pull origin main

# Install and build
npm install
npm run build

# Restart everything
pm2 restart ecosystem.config.js
```

## Health Checks

### API Health Check

```bash
# Check API is responding
curl http://localhost:3000/networks

# Expected response:
# {"success":true,"data":[...]}
```

### Listener Health Check

```bash
# Check logs for recent activity
pm2 logs blockchain-listener --lines 50 | grep -E "cached|Syncing|active"

# Check for errors
pm2 logs blockchain-listener --err --lines 50
```

### Redis Health Check

```bash
# Check Redis connectivity
docker exec universal-listener-redis redis-cli ping
# Expected: PONG

# Check Redis memory usage
docker exec universal-listener-redis redis-cli INFO memory | grep used_memory_human
```

## Troubleshooting

### Application Won't Start

```bash
# Check PM2 logs
pm2 logs blockchain-listener --err --lines 100

# Common issues:
# 1. Redis not running
docker compose up -d

# 2. .env file missing or incorrect
cat .env | grep ALCHEMY_API_KEY

# 3. Build not completed
npm run build

# 4. Port already in use
lsof -i :3000
pm2 delete all
pm2 start ecosystem.config.js
```

### High Memory Usage

```bash
# Check memory usage
pm2 list

# Restart specific app if memory is high
pm2 restart blockchain-listener

# Check logs for memory issues
pm2 logs blockchain-listener | grep -i "memory\|heap"
```

### Redis Connection Issues

```bash
# Check Redis container
docker ps | grep redis

# Restart Redis
docker compose restart

# Check Redis logs
docker logs universal-listener-redis
```

### Rate Limiting Errors

```bash
# Check for Alchemy rate limit errors
pm2 logs blockchain-listener | grep -i "rate\|429\|SERVER_ERROR"

# If seeing many errors:
# 1. Verify you're using paid tier API key
# 2. Check chunk sizes in listener files (should be 10 for ERC20, 10 for native)
# 3. Increase delays between requests if needed
```

## Environment-Specific Configurations

### Development

```bash
# Start with development settings
NODE_ENV=development pm2 start ecosystem.config.js

# Or create ecosystem.config.dev.js with different settings
```

### Staging

```bash
# Use staging environment
NODE_ENV=staging pm2 start ecosystem.config.js
```

### Production

```bash
# Production (default)
pm2 start ecosystem.config.js

# Save process list
pm2 save

# Setup startup script
pm2 startup
```

## Backup and Recovery

### Backup PM2 Configuration

```bash
# Save current PM2 process list
pm2 save

# Backup the dump file
cp ~/.pm2/dump.pm2 ~/pm2-backup-$(date +%Y%m%d).pm2
```

### Restore PM2 Configuration

```bash
# Restore from saved configuration
pm2 resurrect

# Or manually start from ecosystem file
pm2 start ecosystem.config.js
```

## Performance Tuning

### Adjust Memory Limits

Edit `ecosystem.config.js`:

```javascript
{
  name: 'blockchain-listener',
  max_memory_restart: '4G',  // Increase if needed
  // ...
}
```

Then restart:
```bash
pm2 restart ecosystem.config.js
```

### Monitoring with PM2 Plus (Optional)

```bash
# Link to PM2 Plus for advanced monitoring
pm2 link <secret_key> <public_key>
```

## Systemd Integration (Alternative)

If you prefer systemd over PM2 startup:

```bash
# Generate systemd service
pm2 startup systemd

# Follow the instructions, then:
pm2 save

# Manage with systemd
sudo systemctl status pm2-ubuntu
sudo systemctl restart pm2-ubuntu
```

## Security Best Practices

1. **Protect .env file**
   ```bash
   chmod 600 .env
   ```

2. **Run as non-root user**
   ```bash
   # PM2 should be run as non-root user (e.g., ubuntu)
   # Avoid running as root
   ```

3. **Firewall configuration**
   ```bash
   # Only expose necessary ports
   sudo ufw allow 3000/tcp  # API port
   sudo ufw enable
   ```

4. **Use reverse proxy**
   - See PRODUCTION_DEPLOY.md for Nginx configuration

## Quick Reference

```bash
# Start everything
pm2 start ecosystem.config.js

# View status
pm2 list

# View logs
pm2 logs

# Monitor
pm2 monit

# Restart all
pm2 restart all

# Save configuration
pm2 save

# Stop everything
pm2 stop all
```

## Production Checklist

- [ ] PM2 installed globally
- [ ] Redis container running
- [ ] .env file configured with paid tier API key
- [ ] Project built (`npm run build`)
- [ ] Logs directory created
- [ ] PM2 processes started
- [ ] PM2 configuration saved (`pm2 save`)
- [ ] PM2 startup configured (`pm2 startup`)
- [ ] Health checks passing
- [ ] Log rotation configured
- [ ] Monitoring setup (optional)

## Support

If you encounter issues:

1. Check PM2 logs: `pm2 logs --lines 200`
2. Check Redis: `docker logs universal-listener-redis`
3. Verify .env configuration
4. Review PRODUCTION_READY.md and RELIABILITY.md

---

**Last Updated**: 2025-12-31
**PM2 Version**: Compatible with PM2 v5.x+
