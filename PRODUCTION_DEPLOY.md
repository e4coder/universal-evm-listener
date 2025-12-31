# Production Deployment Guide

## Pre-Deployment Checklist

### ✅ Development Complete
- [x] All 13 networks configured
- [x] ERC20 and native transfer listeners implemented
- [x] Redis caching with 1-hour TTL (configurable)
- [x] REST API with 10 endpoints
- [x] End-to-end testing completed
- [x] Documentation complete

### ✅ Local Testing Verified
- [x] TypeScript compilation successful
- [x] Redis connectivity tested
- [x] Cache storage/retrieval working
- [x] API endpoints responding correctly
- [x] Live blockchain data captured
- [x] Graceful shutdown working

## Production Deployment Steps

### 1. Prepare Production Environment

```bash
# SSH into production server
ssh your-production-server

# Clone the repository
git clone <your-repo-url>
cd universal_listener

# Or if already cloned, pull latest
git pull origin main
```

### 2. Install Dependencies

```bash
# Install Node.js (if not already installed)
# Requires Node.js v18 or higher
node --version  # Should be >= 18

# Install project dependencies
npm install
```

### 3. Configure Environment Variables

```bash
# Copy environment template
cp .env.example .env

# Edit .env with production values
nano .env
```

**Production Environment Variables:**
```env
# REQUIRED: Your Alchemy API key
ALCHEMY_API_KEY=your_production_alchemy_key

# REQUIRED: Redis connection URL
# For production, use your hosted Redis (e.g., Redis Cloud, AWS ElastiCache)
REDIS_URL=redis://your-redis-host:6379
# Or with auth:
# REDIS_URL=redis://:password@your-redis-host:6379

# Cache TTL in hours (default: 1)
CACHE_TTL_HOURS=1

# API Server Port (default: 3000)
API_PORT=5459
```

### 4. Set Up Redis (Production)

**Option A: Docker (Simple)**
```bash
docker compose up -d
```

**Option B: Managed Redis (Recommended for Production)**
- [Redis Cloud](https://redis.com/cloud/)
- [AWS ElastiCache](https://aws.amazon.com/elasticache/)
- [Upstash](https://upstash.com/)
- [DigitalOcean Managed Redis](https://www.digitalocean.com/products/managed-databases)

Update `REDIS_URL` in `.env` with your hosted Redis connection string.

### 5. Build the Project

```bash
npm run build
```

Verify build output in `dist/` directory:
```bash
ls -la dist/
```

### 6. Run as Production Service

**Option A: Using PM2 (Recommended)**

Install PM2:
```bash
npm install -g pm2
```

Start the listener:
```bash
pm2 start dist/index.js --name blockchain-listener
```

Start the API server:
```bash
pm2 start dist/api/server.js --name api-server
```

Configure PM2 to restart on reboot:
```bash
pm2 startup
pm2 save
```

View logs:
```bash
pm2 logs blockchain-listener
pm2 logs api-server
```

Monitor processes:
```bash
pm2 status
pm2 monit
```

**Option B: Using systemd**

Create systemd service file:
```bash
sudo nano /etc/systemd/system/blockchain-listener.service
```

```ini
[Unit]
Description=Universal Blockchain Listener
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/universal_listener
Environment=NODE_ENV=production
ExecStart=/usr/bin/node dist/index.js
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Create API server service:
```bash
sudo nano /etc/systemd/system/api-server.service
```

```ini
[Unit]
Description=Blockchain API Server
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/universal_listener
Environment=NODE_ENV=production
ExecStart=/usr/bin/node dist/api/server.js
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable and start services:
```bash
sudo systemctl enable blockchain-listener
sudo systemctl enable api-server
sudo systemctl start blockchain-listener
sudo systemctl start api-server
```

Check status:
```bash
sudo systemctl status blockchain-listener
sudo systemctl status api-server
```

View logs:
```bash
sudo journalctl -u blockchain-listener -f
sudo journalctl -u api-server -f
```

### 7. Set Up Nginx (Optional - API Reverse Proxy)

If you want to expose the API publicly:

```bash
sudo apt install nginx
sudo nano /etc/nginx/sites-available/blockchain-api
```

```nginx
server {
    listen 80;
    server_name your-domain.com;

    location / {
        proxy_pass http://localhost:5459;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

Enable site:
```bash
sudo ln -s /etc/nginx/sites-available/blockchain-api /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl restart nginx
```

### 8. Set Up SSL/TLS (Optional but Recommended)

Using Let's Encrypt:
```bash
sudo apt install certbot python3-certbot-nginx
sudo certbot --nginx -d your-domain.com
```

### 9. Configure Firewall

```bash
# Allow SSH
sudo ufw allow 22/tcp

# Allow HTTP/HTTPS (if using Nginx)
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp

# Allow API port (if not using Nginx)
sudo ufw allow 3000/tcp

# Enable firewall
sudo ufw enable
sudo ufw status
```

## Monitoring & Maintenance

### Health Checks

Create a monitoring script:
```bash
#!/bin/bash
# check-health.sh

# Check if listener is running
if pm2 list | grep -q "blockchain-listener.*online"; then
    echo "✅ Listener is running"
else
    echo "❌ Listener is down"
    pm2 restart blockchain-listener
fi

# Check if API is running
if pm2 list | grep -q "api-server.*online"; then
    echo "✅ API server is running"
else
    echo "❌ API server is down"
    pm2 restart api-server
fi

# Check Redis
if redis-cli ping | grep -q "PONG"; then
    echo "✅ Redis is running"
else
    echo "❌ Redis is down"
fi

# Check API endpoint
if curl -s http://localhost:5459/networks | grep -q "success"; then
    echo "✅ API is responding"
else
    echo "❌ API is not responding"
fi
```

Add to crontab:
```bash
chmod +x check-health.sh
crontab -e
```

Add line:
```
*/5 * * * * /path/to/check-health.sh >> /var/log/health-check.log 2>&1
```

### View Logs

PM2:
```bash
pm2 logs blockchain-listener --lines 100
pm2 logs api-server --lines 100
```

systemd:
```bash
sudo journalctl -u blockchain-listener -n 100 -f
sudo journalctl -u api-server -n 100 -f
```

### Update Application

```bash
# Pull latest code
git pull origin main

# Install dependencies (if changed)
npm install

# Rebuild
npm run build

# Restart services
pm2 restart all
# OR
sudo systemctl restart blockchain-listener
sudo systemctl restart api-server
```

## Performance Tuning

### Redis Configuration

For high-throughput production, edit Redis config:
```bash
# If using Docker
docker exec universal-listener-redis redis-cli CONFIG SET maxmemory 2gb
docker exec universal-listener-redis redis-cli CONFIG SET maxmemory-policy allkeys-lru
```

### Node.js Optimization

Increase memory limit if needed:
```bash
pm2 start dist/index.js --name blockchain-listener --max-memory-restart 2G
```

### Alchemy Rate Limits

Monitor your Alchemy usage:
- Free tier: 300M compute units/month
- Growth plan: Consider upgrading for production

## Backup & Disaster Recovery

### Redis Backups

Configure Redis persistence:
```bash
# Enable AOF (Append Only File)
redis-cli CONFIG SET appendonly yes
```

### Application Backups

```bash
# Backup configuration
cp .env .env.backup-$(date +%Y%m%d)

# Git backup
git commit -am "Production configuration"
git push origin main
```

## Security Checklist

- [ ] `.env` file has secure permissions (`chmod 600 .env`)
- [ ] Alchemy API key is kept secret
- [ ] Redis is password-protected (if exposed)
- [ ] Firewall is configured
- [ ] SSL/TLS enabled for API (if public)
- [ ] Regular security updates (`sudo apt update && sudo apt upgrade`)
- [ ] API rate limiting enabled (if needed)

## Troubleshooting

### Listener not capturing events
```bash
# Check Alchemy API key
grep ALCHEMY_API_KEY .env

# Check network connectivity
curl -I https://dashboard.alchemy.com

# Check logs for errors
pm2 logs blockchain-listener --err
```

### Redis connection errors
```bash
# Test Redis connection
redis-cli -h your-redis-host -p 6379 ping

# Check Redis URL in .env
grep REDIS_URL .env
```

### API not responding
```bash
# Check if process is running
pm2 status api-server

# Check port is not in use
sudo lsof -i :3000

# Test locally
curl http://localhost:5459/networks
```

## Production Metrics

Monitor these metrics:
- **Uptime**: Should be 99.9%+
- **Event capture rate**: Varies by network activity
- **API response time**: < 100ms
- **Redis memory usage**: Monitor with `redis-cli INFO memory`
- **CPU usage**: Should be < 20% normally
- **Memory usage**: ~ 100MB per network listener

## Cost Estimation

- **Alchemy**: Free tier supports moderate usage
- **Redis**:
  - Docker (self-hosted): Free
  - Redis Cloud: $0-$50/month depending on memory
- **Server**:
  - DigitalOcean Droplet: $6-$12/month
  - AWS EC2: $10-$30/month

## Support

For issues:
1. Check logs first
2. Review [README.md](README.md)
3. Check [TEST_RESULTS.md](TEST_RESULTS.md)
4. Open GitHub issue

## Success Criteria

Your deployment is successful when:
- ✅ All 13 network listeners are running
- ✅ Transfers are being cached to Redis
- ✅ API endpoints return cached data
- ✅ Services auto-restart on failure
- ✅ Logs are clean (no persistent errors)
- ✅ Cache TTL is working (verify with Redis TTL command)

## Next Steps After Deployment

1. Monitor logs for 24 hours
2. Test API endpoints from external client
3. Verify cache expiration after TTL
4. Set up automated alerts
5. Document any production-specific configurations
6. Create runbook for common operations
