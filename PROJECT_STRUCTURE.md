# Project Structure

```
universal_listener/
├── src/
│   ├── api/
│   │   └── server.ts              # HTTP API server for querying cached data
│   ├── cache/
│   │   └── redis.ts               # Redis connection & caching utilities
│   ├── config/
│   │   └── networks.ts            # Network configurations (13 chains)
│   ├── listeners/
│   │   ├── erc20Listener.ts       # ERC20 Transfer event listener
│   │   └── nativeListener.ts      # Native token transfer listener
│   ├── services/
│   │   └── queryService.ts        # Query utilities for cached transfers
│   ├── types/
│   │   └── index.ts               # TypeScript type definitions
│   └── index.ts                   # Main application entry point
├── examples/
│   └── query-example.ts           # Example script for querying data
├── .env.example                   # Environment variable template
├── .gitignore                     # Git ignore rules
├── docker-compose.yml             # Docker Compose for Redis
├── package.json                   # NPM dependencies and scripts
├── tsconfig.json                  # TypeScript configuration
├── README.md                      # Main documentation
├── SETUP.md                       # Quick setup guide
└── PROJECT_STRUCTURE.md           # This file
```

## File Descriptions

### Core Application Files

#### [src/index.ts](src/index.ts)
Main entry point that:
- Initializes Redis connection
- Creates Alchemy instances for all 13 networks
- Starts ERC20 and native transfer listeners for each network
- Handles graceful shutdown

#### [src/cache/redis.ts](src/cache/redis.ts)
Redis caching layer that:
- Manages Redis connection
- Stores ERC20 and native transfers with 24-hour TTL
- Creates indexes by sender (`from`), receiver (`to`), and both
- Provides query methods for retrieving cached transfers

#### [src/config/networks.ts](src/config/networks.ts)
Network configurations including:
- All 13 supported blockchain networks
- Chain IDs, network names, Alchemy network enums
- Native token symbols (ETH, MATIC, BNB, etc.)

### Listeners

#### [src/listeners/erc20Listener.ts](src/listeners/erc20Listener.ts)
Monitors ERC20 Transfer events:
- Subscribes to Alchemy WebSocket for mined transactions
- Filters for ERC20 Transfer event signature
- Decodes transfer data (from, to, value, token address)
- Caches events to Redis with proper indexing

#### [src/listeners/nativeListener.ts](src/listeners/nativeListener.ts)
Monitors native token transfers:
- Subscribes to Alchemy WebSocket for transactions with value
- Filters transactions with native token transfers
- Extracts sender, receiver, and amount
- Caches to Redis with indexing

### Services

#### [src/services/queryService.ts](src/services/queryService.ts)
High-level query interface:
- Query ERC20 transfers by from/to/both addresses
- Query native transfers by from/to/both addresses
- Get all transfers for an address (combining sent & received)
- Deduplication and sorting by timestamp

### API

#### [src/api/server.ts](src/api/server.ts)
HTTP REST API server:
- Exposes query endpoints for cached data
- RESTful routes for ERC20 and native transfers
- CORS-enabled for web client access
- JSON response format

### Types

#### [src/types/index.ts](src/types/index.ts)
TypeScript interfaces:
- `ERC20Transfer`: ERC20 transfer event structure
- `NativeTransfer`: Native token transfer structure
- `AllTransfers`: Combined transfer response

## Configuration Files

### [package.json](package.json)
NPM configuration with:
- Dependencies (alchemy-sdk, redis, ethers)
- Dev dependencies (TypeScript, ts-node)
- Scripts for build, dev, and production

### [tsconfig.json](tsconfig.json)
TypeScript compiler configuration:
- Target ES2022
- CommonJS modules
- Strict type checking
- Source maps for debugging

### [docker-compose.yml](docker-compose.yml)
Docker configuration:
- Redis 7 Alpine image
- Port mapping (6379)
- Persistent volume for data
- Auto-restart policy

### [.env.example](.env.example)
Environment template:
- Alchemy API key
- Redis connection URL
- Optional network-specific keys

## Documentation Files

### [README.md](README.md)
Main documentation covering:
- Feature overview
- Architecture explanation
- Installation instructions
- API endpoint documentation
- Usage examples

### [SETUP.md](SETUP.md)
Quick start guide:
- Step-by-step setup instructions
- Prerequisites and dependencies
- Common troubleshooting
- Development mode

### [PROJECT_STRUCTURE.md](PROJECT_STRUCTURE.md)
This file - complete project structure reference

## Examples

### [examples/query-example.ts](examples/query-example.ts)
Demonstrates:
- Connecting to Redis
- Querying transfers for specific addresses
- Different query patterns (by direction, network)
- Displaying results

## Data Flow

```
Blockchain Networks (13)
    ↓
Alchemy WebSocket Subscriptions
    ↓
Event Listeners (ERC20 & Native)
    ↓
Redis Cache (24h TTL)
    ↓
Query Service
    ↓
API Server / Direct Queries
    ↓
End Users / Applications
```

## Key Features by File

| Feature | Primary File(s) |
|---------|----------------|
| Multi-chain monitoring | [src/index.ts](src/index.ts), [src/config/networks.ts](src/config/networks.ts) |
| ERC20 event listening | [src/listeners/erc20Listener.ts](src/listeners/erc20Listener.ts) |
| Native transfer listening | [src/listeners/nativeListener.ts](src/listeners/nativeListener.ts) |
| Redis caching | [src/cache/redis.ts](src/cache/redis.ts) |
| Transfer queries | [src/services/queryService.ts](src/services/queryService.ts) |
| HTTP API | [src/api/server.ts](src/api/server.ts) |
| Type safety | [src/types/index.ts](src/types/index.ts) |
