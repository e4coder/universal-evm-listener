# Quick Setup Guide

## 1. Prerequisites

Make sure you have the following installed:
- Node.js v18+ ([Download](https://nodejs.org/))
- Docker & Docker Compose ([Download](https://docs.docker.com/get-docker/))
- Git

## 2. Get Alchemy API Key

1. Go to [https://dashboard.alchemy.com/](https://dashboard.alchemy.com/)
2. Sign up or log in
3. Create a new app
4. Copy your API key

## 3. Setup Redis

Using Docker (recommended):
```bash
docker-compose up -d
```

Or install Redis locally:
- macOS: `brew install redis && brew services start redis`
- Ubuntu: `sudo apt install redis-server && sudo systemctl start redis`
- Windows: Use Docker or [WSL2](https://redis.io/docs/getting-started/installation/install-redis-on-windows/)

## 4. Install Dependencies

```bash
npm install
```

## 5. Configure Environment

```bash
cp .env.example .env
```

Edit [.env](.env) and configure:
```env
ALCHEMY_API_KEY=your_actual_api_key_here
REDIS_URL=redis://localhost:6379
CACHE_TTL_HOURS=1  # Optional: Cache duration in hours (default: 1)
```

## 6. Build the Project

```bash
npm run build
```

## 7. Run the Application

### Option A: Run the Listener Only

This will start monitoring all 13 blockchain networks and cache events to Redis:

```bash
npm start
```

### Option B: Run the API Server Only

If the listener is already running, you can start the API server to query cached data:

```bash
npm run api
```

### Option C: Run Both

In separate terminal windows:

Terminal 1 - Listener:
```bash
npm start
```

Terminal 2 - API Server:
```bash
npm run api
```

## 8. Test the API

Once the API server is running, test it:

```bash
# Get list of supported networks
curl http://localhost:3000/networks

# Get all transfers for an address on Ethereum (chainId: 1)
curl http://localhost:3000/all/1/0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb

# Get ERC20 transfers on Polygon (chainId: 137)
curl http://localhost:3000/erc20/address/137/0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

## 9. Monitor Logs

The listener will output logs showing:
- Connection status to Redis
- Network initialization
- Transfer events being cached

Example output:
```
🚀 Starting Universal Blockchain Listener...
📡 Monitoring 13 networks
✅ Redis connected
✅ [Ethereum] Listeners started successfully
✅ [Polygon] Listeners started successfully
...
[Ethereum] ERC20 Transfer cached: 0x123... -> 0x456... (Token: 0xabc...)
[Polygon] Native Transfer cached: 0x789... -> 0xdef... (MATIC)
```

## Troubleshooting

### Redis Connection Error
- Make sure Redis is running: `docker-compose ps` or `redis-cli ping`
- Check [.env](.env) has correct `REDIS_URL`

### Alchemy API Error
- Verify your API key in [.env](.env)
- Check you haven't exceeded rate limits
- Ensure your Alchemy plan supports all networks

### Build Errors
- Delete `node_modules` and run `npm install` again
- Make sure you're using Node.js v18+: `node --version`

### No Data in Cache
- Wait a few minutes for transactions to occur on the networks
- The listener only caches NEW transactions after it starts
- Check listener logs for errors

## Development Mode

Run without building:

```bash
# Listener
npm run dev

# API Server
npm run api:dev
```

## Stopping the Services

```bash
# Stop the Node.js app
Ctrl + C

# Stop Redis (if using Docker)
docker-compose down
```

## Next Steps

- Read the full [README.md](README.md) for API documentation
- Check [examples/query-example.ts](examples/query-example.ts) for programmatic usage
- Customize network configurations in [src/config/networks.ts](src/config/networks.ts)
