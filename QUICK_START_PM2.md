# Quick Start - PM2 Deployment

## One-Command Deployment

```bash
./deploy.sh
```

That's it! The script will:
- ✅ Install PM2 if needed
- ✅ Check .env configuration
- ✅ Install dependencies
- ✅ Build the project
- ✅ Start Redis
- ✅ Launch both apps with PM2
- ✅ Save PM2 configuration
- ✅ Run health checks

## Manual Deployment (Alternative)

```bash
# 1. Install PM2 globally
npm install -g pm2

# 2. Setup environment
cp .env.example .env
nano .env  # Add ALCHEMY_API_KEY

# 3. Install and build
npm install
npm run build

# 4. Start Redis
docker compose up -d

# 5. Start apps with PM2
npm run pm2:start

# 6. Save and setup auto-start
pm2 save
pm2 startup
```

## Daily Operations

```bash
# View status
npm run pm2:status

# View logs (live)
npm run pm2:logs

# Monitor resources
npm run pm2:monit

# Restart apps (zero downtime)
npm run pm2:reload

# Stop apps
npm run pm2:stop

# Delete apps
npm run pm2:delete
```

## After Code Updates

```bash
# Pull latest code
git pull

# Build and reload
npm run build
npm run pm2:reload
```

## Troubleshooting

```bash
# Check logs for errors
pm2 logs --err --lines 100

# Restart if needed
npm run pm2:restart

# Full reset
npm run pm2:delete
./deploy.sh
```

## API Endpoints

Once deployed, access your API at:

```bash
# Check all networks
curl http://localhost:5459/networks

# Query transfers (example: Arbitrum)
curl http://localhost:5459/all/42161/YOUR_ADDRESS
```

## PM2 Dashboard (Optional)

For web-based monitoring:

```bash
# Install PM2 Web
pm2 install pm2-server-monit

# Access at http://your-server:9615
```

## Health Monitoring

```bash
# Quick health check
curl http://localhost:5459/networks && echo "✅ API OK" || echo "❌ API Down"

# Check Redis
docker exec universal-listener-redis redis-cli ping

# Check PM2 processes
pm2 list
```

## Production Checklist

- [x] PM2 ecosystem.config.js created
- [x] Deploy script created and executable
- [x] NPM scripts added for PM2 commands
- [x] Logs directory created
- [x] .gitignore updated
- [ ] PM2 installed on server
- [ ] .env configured with paid API key
- [ ] Apps deployed with PM2
- [ ] PM2 startup configured
- [ ] Health checks passing

## Next Steps

1. Test locally: `./deploy.sh`
2. Verify health: `npm run pm2:status`
3. Push to git: `git push`
4. Deploy to production server
5. Configure PM2 startup: `pm2 startup`

---

**See PM2_DEPLOYMENT.md for complete documentation**
