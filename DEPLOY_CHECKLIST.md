# Production Deployment Checklist

Complete this checklist before deploying to production.

## Pre-Deployment

### Local Testing
- [ ] All services stopped locally
- [ ] Code builds without errors (`npm run build`)
- [ ] Tests pass (if applicable)
- [ ] No console errors in development

### Configuration
- [ ] `.env` file has production API key (Alchemy paid tier)
- [ ] `CACHE_TTL_HOURS` set to desired value (default: 1)
- [ ] `REDIS_URL` configured correctly
- [ ] `API_PORT` set (default: 3000)

### Code Review
- [ ] All sensitive data removed from code
- [ ] No hardcoded credentials
- [ ] No debug/console.log statements in production code
- [ ] Git repository clean (`git status`)

## Deployment

### Server Setup
- [ ] Production server provisioned
- [ ] Node.js v18+ installed
- [ ] Docker and Docker Compose installed
- [ ] PM2 installed globally (`npm install -g pm2`)
- [ ] Firewall configured (port 5459 for API)

### Code Deployment
- [ ] Repository cloned or pulled to server
- [ ] Dependencies installed (`npm install`)
- [ ] Project built (`npm run build`)
- [ ] `.env` file created and configured

### Services
- [ ] Redis started (`docker compose up -d`)
- [ ] Redis accessible (`docker exec universal-listener-redis redis-cli ping`)
- [ ] PM2 apps started (`./deploy.sh` or `npm run pm2:start`)
- [ ] PM2 configuration saved (`pm2 save`)

### Auto-Start
- [ ] PM2 startup configured (`pm2 startup`)
- [ ] Startup command executed (as shown by pm2 startup)
- [ ] Server rebooted to test auto-start (optional but recommended)

## Post-Deployment

### Health Checks
- [ ] API responding (`curl http://localhost:5459/networks`)
- [ ] All 13 networks initialized (check logs: `npm run pm2:logs`)
- [ ] No critical errors in logs
- [ ] Redis connected and caching data

### Monitoring
- [ ] PM2 processes running (`pm2 list`)
- [ ] Memory usage acceptable (<2GB per listener)
- [ ] CPU usage reasonable (<50% steady state)
- [ ] Logs being written to `logs/` directory

### Functionality
- [ ] Send test transfer on one network
- [ ] Wait 30 seconds
- [ ] Query API for test transfer
- [ ] Verify transfer appears in results

### Security
- [ ] `.env` file permissions set (`chmod 600 .env`)
- [ ] Running as non-root user
- [ ] Firewall rules applied
- [ ] SSL/TLS configured (if exposing API publicly)
- [ ] Rate limiting configured (if exposing API publicly)

### Documentation
- [ ] Team knows how to access logs (`npm run pm2:logs`)
- [ ] Team knows how to restart (`npm run pm2:restart`)
- [ ] Emergency contacts documented
- [ ] Alchemy API key backed up securely

## Maintenance

### Daily
- [ ] Check PM2 status (`pm2 list`)
- [ ] Monitor error logs (`pm2 logs --err --lines 50`)
- [ ] Verify Redis is running

### Weekly
- [ ] Review memory usage trends
- [ ] Check for Alchemy rate limit warnings
- [ ] Archive old logs if needed
- [ ] Check for app updates/dependencies

### Monthly
- [ ] Review and optimize chunk sizes if needed
- [ ] Check Alchemy usage/billing
- [ ] Update dependencies if security patches available
- [ ] Review and clean Redis if needed

## Rollback Plan

If issues occur:

1. **Quick Rollback**
   ```bash
   pm2 stop all
   git checkout <previous-commit>
   npm run build
   npm run pm2:restart
   ```

2. **Full Reset**
   ```bash
   pm2 delete all
   docker compose down
   docker compose up -d
   ./deploy.sh
   ```

3. **Emergency Stop**
   ```bash
   pm2 stop all
   docker compose down
   ```

## Support Contacts

- **Developer**: [Your contact]
- **DevOps**: [DevOps contact]
- **Alchemy Support**: https://www.alchemy.com/support

## Notes

- Deployment Date: _______________
- Deployed By: _______________
- Server: _______________
- Alchemy Plan: _______________
- Git Commit: _______________

---

**Last Updated**: 2025-12-31
