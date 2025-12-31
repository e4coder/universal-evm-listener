# Changelog

## [1.0.0] - 2025-12-31

### Initial Release

#### Features Implemented
- ✅ Multi-chain blockchain event listener for 13 networks
- ✅ ERC20 token transfer monitoring and caching
- ✅ Native token transfer monitoring and caching
- ✅ Redis-based caching with automatic expiration
- ✅ Configurable cache TTL (default: 1 hour)
- ✅ Query service for retrieving transfers by address
- ✅ REST API server for querying cached data
- ✅ Full TypeScript implementation
- ✅ Docker Compose setup for Redis
- ✅ Comprehensive documentation

#### Supported Networks (13)
1. Ethereum (Chain ID: 1)
2. Arbitrum One (Chain ID: 42161)
3. Polygon (Chain ID: 137)
4. OP Mainnet (Chain ID: 10)
5. Base (Chain ID: 8453)
6. Gnosis (Chain ID: 100)
7. BNB Smart Chain (Chain ID: 56)
8. Avalanche (Chain ID: 43114)
9. Linea Mainnet (Chain ID: 59144)
10. Unichain (Chain ID: 130)
11. Soneium Mainnet (Chain ID: 1868)
12. Sonic (Chain ID: 146)
13. Ink (Chain ID: 57073)

#### Configuration Options
- `ALCHEMY_API_KEY` - Alchemy API key (required)
- `REDIS_URL` - Redis connection URL (default: redis://localhost:6379)
- `CACHE_TTL_HOURS` - Cache duration in hours (default: 1)
- `API_PORT` - API server port (default: 3000)

#### API Endpoints
- `GET /networks` - List all supported networks
- `GET /erc20/from/:chainId/:address` - ERC20 transfers from address
- `GET /erc20/to/:chainId/:address` - ERC20 transfers to address
- `GET /erc20/both/:chainId/:from/:to` - ERC20 transfers between addresses
- `GET /erc20/address/:chainId/:address` - All ERC20 transfers for address
- `GET /native/from/:chainId/:address` - Native transfers from address
- `GET /native/to/:chainId/:address` - Native transfers to address
- `GET /native/both/:chainId/:from/:to` - Native transfers between addresses
- `GET /native/address/:chainId/:address` - All native transfers for address
- `GET /all/:chainId/:address` - All transfers (ERC20 + native) for address

#### Project Structure
```
src/
├── api/server.ts              # HTTP API server
├── cache/redis.ts             # Redis caching utilities
├── config/networks.ts         # Network configurations
├── listeners/
│   ├── erc20Listener.ts       # ERC20 event listener
│   └── nativeListener.ts      # Native transfer listener
├── services/queryService.ts   # Query service
├── types/index.ts             # Type definitions
└── index.ts                   # Main entry point
```

#### Documentation Files
- `README.md` - Main documentation
- `SETUP.md` - Quick setup guide
- `FEATURES.md` - Detailed feature list
- `PROJECT_STRUCTURE.md` - Architecture overview
- `CHANGELOG.md` - This file

#### Recent Updates (Latest)
- Changed default cache TTL from 24 hours to 1 hour
- Made cache TTL configurable via `CACHE_TTL_HOURS` environment variable
- Added TTL logging on application startup
- Updated all documentation to reflect configurable cache duration

### What's Working
✅ All core functionality implemented and tested
✅ Multi-network support (13 chains)
✅ Real-time event listening via Alchemy WebSocket
✅ Redis caching with indexing
✅ Query API with multiple endpoints
✅ TypeScript compilation
✅ Environment configuration
✅ Docker setup for Redis

### Known Limitations
- Only captures events after the listener starts (no historical backfill)
- Subject to Alchemy API rate limits
- Cache duration limited by Redis memory
- WebSocket connection stability depends on Alchemy

### Next Steps for Users
1. Install dependencies: `npm install`
2. Start Redis: `docker-compose up -d`
3. Configure environment: Copy `.env.example` to `.env` and add Alchemy API key
4. Build project: `npm run build`
5. Start listener: `npm start`
6. Start API (optional): `npm run api`

### Future Enhancement Ideas
- [ ] Add support for ERC721/ERC1155 (NFT) transfers
- [ ] Implement historical backfilling
- [ ] Add GraphQL API
- [ ] Create web dashboard
- [ ] Add webhook notifications
- [ ] Implement metrics/monitoring
- [ ] Add database persistence option
- [ ] Support custom RPC endpoints
- [ ] Add transfer value calculation in USD
- [ ] Implement custom event filters
