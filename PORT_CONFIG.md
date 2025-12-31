# API Port Configuration

## Production API Port: 5459

The production API server runs on port **5459**.

### Configuration Files

All configuration files have been updated:

- **ecosystem.config.js** - PM2 configuration sets `API_PORT=5459`
- **.env.example** - Template shows `API_PORT=5459`
- **deploy.sh** - Health checks use port 5459
- **All documentation** - Updated with correct port

### API Endpoints

Access the API at:

```bash
# Base URL
http://localhost:5459

# Health check
curl http://localhost:5459/networks

# Query example (Arbitrum)
curl http://localhost:5459/all/42161/YOUR_ADDRESS
```

### Firewall Configuration

Make sure to allow port 5459:

```bash
# UFW (Ubuntu)
sudo ufw allow 5459/tcp

# Or if using iptables
sudo iptables -A INPUT -p tcp --dport 5459 -j ACCEPT
```

### Nginx Reverse Proxy (Optional)

If using Nginx as reverse proxy:

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
    }
}
```

### Environment Variables

The API port can be configured via:

1. **.env file** (recommended)
   ```bash
   API_PORT=5459
   ```

2. **PM2 ecosystem.config.js**
   ```javascript
   env: {
     API_PORT: 5459,
   }
   ```

3. **Command line** (for testing)
   ```bash
   API_PORT=5459 npm run api
   ```

### Monitoring

Updated monitoring scripts use port 5459:

```bash
# Monitor wallet transfers
./monitor-simple.sh

# Monitor specific wallet
./monitor-wallet.sh
```

All queries will automatically use port 5459.

---

**Last Updated**: 2025-12-31
**Production Port**: 5459
