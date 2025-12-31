# Universal Blockchain Listener

A multi-chain blockchain event listener that monitors and caches ERC20 token transfers and native token transfers across 13 different blockchain networks using Alchemy and Redis.

## Features

- **Multi-Chain Support**: Monitors 13 blockchain networks simultaneously
  - Ethereum, Arbitrum One, Polygon, OP Mainnet, Base
  - Gnosis, BNB Smart Chain, Avalanche, Linea
  - Unichain, Soneium, Sonic, Ink

- **ERC20 Token Transfers**: Captures all ERC20 Transfer events
- **Native Token Transfers**: Monitors native currency transfers (ETH, MATIC, BNB, etc.)
- **Redis Caching**: Stores transfer data with configurable TTL (default: 1 hour)
- **Flexible Queries**: Query transfers by sender, receiver, or both
- **REST API**: Built-in HTTP API for querying cached data

## Architecture

```
src/
├── cache/
│   └── redis.ts          # Redis connection and caching logic
├── config/
│   └── networks.ts       # Network configurations for all chains
├── listeners/
│   ├── erc20Listener.ts  # ERC20 Transfer event listener
│   └── nativeListener.ts # Native transfer listener
├── services/
│   └── queryService.ts   # Query utilities for cached data
├── api/
│   └── server.ts         # HTTP API server
└── index.ts              # Main application entry point
```

## Prerequisites

- Node.js v18 or higher
- Redis server running locally or remotely
- Alchemy API key (get one at https://dashboard.alchemy.com/)

## Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd universal_listener
```

2. Install dependencies:
```bash
npm install
```

3. Create environment configuration:
```bash
cp .env.example .env
```

4. Edit [.env](.env) and configure:
```env
ALCHEMY_API_KEY=your_alchemy_api_key_here
REDIS_URL=redis://localhost:6379
CACHE_TTL_HOURS=1  # Cache duration in hours (default: 1)
```

## Usage

### Running the Listener

Build and start the blockchain listener:

```bash
npm run build
npm start
```

Or run in development mode:

```bash
npm run dev
```

The listener will start monitoring all 13 networks and cache transfer events to Redis.

### Running the API Server

To query cached data via HTTP API:

```bash
npm run build
node dist/api/server.js
```

The API server will start on port 3000 (configurable via `API_PORT` environment variable).

## API Endpoints

### Get Supported Networks
```
GET /networks
```

Returns list of all supported networks with their chain IDs.

### ERC20 Transfer Queries

Get ERC20 transfers FROM an address:
```
GET /erc20/from/:chainId/:address
```

Get ERC20 transfers TO an address:
```
GET /erc20/to/:chainId/:address
```

Get ERC20 transfers between two specific addresses:
```
GET /erc20/both/:chainId/:fromAddress/:toAddress
```

Get all ERC20 transfers for an address (sent or received):
```
GET /erc20/address/:chainId/:address
```

### Native Transfer Queries

Get native transfers FROM an address:
```
GET /native/from/:chainId/:address
```

Get native transfers TO an address:
```
GET /native/to/:chainId/:address
```

Get native transfers between two specific addresses:
```
GET /native/both/:chainId/:fromAddress/:toAddress
```

Get all native transfers for an address (sent or received):
```
GET /native/address/:chainId/:address
```

### Combined Queries

Get ALL transfers (both ERC20 and native) for an address:
```
GET /all/:chainId/:address
```

## Example API Usage

Get all transfers for an address on Ethereum (chainId: 1):
```bash
curl http://localhost:3000/all/1/0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

Get ERC20 transfers sent from an address on Polygon (chainId: 137):
```bash
curl http://localhost:3000/erc20/from/137/0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

Get native transfers received by an address on Base (chainId: 8453):
```bash
curl http://localhost:3000/native/to/8453/0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

## Supported Networks

| Network | Chain ID | Native Token |
|---------|----------|--------------|
| Ethereum | 1 | ETH |
| Arbitrum One | 42161 | ETH |
| Polygon | 137 | MATIC |
| OP Mainnet | 10 | ETH |
| Base | 8453 | ETH |
| Gnosis | 100 | xDAI |
| BNB Smart Chain | 56 | BNB |
| Avalanche | 43114 | AVAX |
| Linea Mainnet | 59144 | ETH |
| Unichain | 130 | ETH |
| Soneium Mainnet | 1868 | ETH |
| Sonic | 146 | S |
| Ink | 57073 | ETH |

## Data Structure

### ERC20 Transfer Object
```typescript
{
  txHash: string;
  token: string;        // Token contract address
  from: string;         // Sender address
  to: string;           // Receiver address
  value: string;        // Transfer amount (hex)
  blockNumber: number;
  timestamp: number;
  chainId: number;
}
```

### Native Transfer Object
```typescript
{
  txHash: string;
  from: string;         // Sender address
  to: string;           // Receiver address
  value: string;        // Transfer amount (hex)
  blockNumber: number;
  timestamp: number;
  chainId: number;
}
```

## Caching Strategy

- **TTL**: Configurable cache expiration (default: 1 hour, set via `CACHE_TTL_HOURS` env variable)
- **Indexing**: Transfers are indexed by:
  - Sender address (`from`)
  - Receiver address (`to`)
  - Both sender and receiver addresses
- **Storage**: Uses Redis sorted sets for time-based ordering

## Development

Build the project:
```bash
npm run build
```

Clean build artifacts:
```bash
npm run clean
```

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
