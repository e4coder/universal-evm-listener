# ERC20 Cache API Documentation

**Base URL:** `https://erc20cache.bitxpay.com`
**Version:** 2.0
**Last Updated:** January 2026

---

## Table of Contents

1. [Overview](#overview)
2. [Authentication](#authentication)
3. [Response Format](#response-format)
4. [Supported Networks](#supported-networks)
5. [Endpoints](#endpoints)
   - [System](#system-endpoints)
   - [ERC20 Transfers](#erc20-transfer-endpoints)
   - [Streaming & Batch (NEW)](#streaming--batch-endpoints)
   - [Fusion+ Swaps](#fusion-swap-endpoints)
   - [Fusion Swaps](#fusion-swap-endpoints-1)
   - [Crypto2Fiat Events](#crypto2fiat-endpoints)
6. [Data Types](#data-types)
7. [Error Handling](#error-handling)
8. [Rate Limits](#rate-limits)
9. [Examples](#examples)

---

## Overview

The ERC20 Cache API provides real-time access to ERC20 token transfers across 13 blockchain networks. It includes specialized support for:

- **ERC20 Transfers** - Standard token transfers
- **Fusion+ Swaps** - Cross-chain atomic swaps via 1inch Fusion+
- **Fusion Swaps** - Single-chain limit orders via 1inch Fusion
- **Crypto2Fiat Events** - Fiat off-ramp transactions

### Key Features

- Real-time indexing (< 5 second latency)
- 13 supported chains
- Cursor-based pagination for efficient polling
- Batch queries for multiple addresses
- Swap type classification on all transfers

---

## Authentication

**No authentication required.** The API is publicly accessible.

CORS is enabled for all origins.

---

## Response Format

All responses follow this structure:

```json
{
  "success": true,
  "data": { ... }
}
```

Error responses:

```json
{
  "success": false,
  "error": "Error message describing what went wrong"
}
```

---

## Supported Networks

| Chain ID | Network | Native Token |
|----------|---------|--------------|
| 1 | Ethereum | ETH |
| 10 | OP Mainnet | ETH |
| 56 | BNB Smart Chain | BNB |
| 100 | Gnosis | xDAI |
| 130 | Unichain | ETH |
| 137 | Polygon | MATIC |
| 146 | Sonic | S |
| 1868 | Soneium Mainnet | ETH |
| 8453 | Base | ETH |
| 42161 | Arbitrum One | ETH |
| 43114 | Avalanche | AVAX |
| 57073 | Ink | ETH |
| 59144 | Linea Mainnet | ETH |

---

## Endpoints

### System Endpoints

#### GET /networks

Returns list of supported blockchain networks.

**Response:**

```json
{
  "success": true,
  "data": [
    {
      "name": "Ethereum",
      "chainId": 1,
      "alchemyNetwork": "eth-mainnet",
      "nativeSymbol": "ETH"
    },
    ...
  ]
}
```

---

#### GET /stats

Returns database statistics.

**Response:**

```json
{
  "success": true,
  "data": {
    "transferCount": 78114,
    "fusionPlusCount": 0,
    "fusionCount": 48,
    "crypto2fiatCount": 0
  }
}
```

---

### ERC20 Transfer Endpoints

#### GET /erc20/from/:chainId/:address

Get transfers **sent from** an address.

**Parameters:**

| Name | Type | Location | Description |
|------|------|----------|-------------|
| chainId | number | path | Chain ID (e.g., 1 for Ethereum) |
| address | string | path | Ethereum address (0x...) |
| limit | number | query | Max results (default: 100) |

**Response:**

```json
{
  "success": true,
  "data": [
    {
      "chainId": 1,
      "txHash": "0x...",
      "token": "0xdac17f958d2ee523a2206206994597c13d831ec7",
      "from": "0x1234...",
      "to": "0xabcd...",
      "value": "1000000",
      "blockNumber": 19000001,
      "timestamp": 1705000100,
      "swapType": null
    }
  ]
}
```

---

#### GET /erc20/to/:chainId/:address

Get transfers **received by** an address.

**Parameters:** Same as `/erc20/from`

---

#### GET /erc20/address/:chainId/:address

Get **all transfers** involving an address (sent or received).

**Parameters:** Same as `/erc20/from`

---

#### GET /erc20/both/:chainId/:from/:to

Get transfers between two specific addresses.

**Parameters:**

| Name | Type | Location | Description |
|------|------|----------|-------------|
| chainId | number | path | Chain ID |
| from | string | path | Sender address |
| to | string | path | Receiver address |
| limit | number | query | Max results (default: 100) |

---

#### GET /erc20/fusion-plus/:chainId/:address

Get transfers tagged as **Fusion+** swaps for an address.

---

#### GET /erc20/fusion-single/:chainId/:address

Get transfers tagged as **Fusion** (single-chain) swaps for an address.

---

#### GET /erc20/crypto2fiat/:chainId/:address

Get transfers tagged as **Crypto2Fiat** events for an address.

---

### Streaming & Batch Endpoints

These endpoints provide efficient cursor-based pagination for real-time polling.

#### GET /erc20/stream/:chainId/:address

Stream transfers with cursor-based pagination using `since_id`.

**Parameters:**

| Name | Type | Location | Required | Description |
|------|------|----------|----------|-------------|
| chainId | number | path | Yes | Chain ID |
| address | string | path | Yes | Ethereum address |
| since_id | number | query | No | Return transfers with id > since_id (default: 0) |
| limit | number | query | No | Max results (default: 100, max: 1000) |
| direction | string | query | No | `"from"`, `"to"`, or `"both"` (default: `"both"`) |

**Response:**

```json
{
  "success": true,
  "data": {
    "transfers": [
      {
        "id": 30010,
        "chainId": 56,
        "txHash": "0x1f63a7a8...",
        "token": "0x93fac02b...",
        "from": "0x00000000d196...",
        "to": "0xf037087c59db...",
        "value": "0x1804f2dd1677ff38",
        "blockNumber": 76912315,
        "timestamp": 1769155435,
        "swapType": "fusion"
      }
    ],
    "nextSinceId": 30010,
    "hasMore": false
  }
}
```

**Response Fields:**

| Field | Type | Description |
|-------|------|-------------|
| transfers | array | Array of transfer objects with `id` field |
| nextSinceId | number | Use this value as `since_id` in next request |
| hasMore | boolean | `true` if more results available |

**Polling Pattern:**

```javascript
let sinceId = 0;

async function poll() {
  const response = await fetch(
    `https://erc20cache.bitxpay.com/erc20/stream/1/${address}?since_id=${sinceId}&limit=100`
  );
  const { data } = await response.json();

  if (data.transfers.length > 0) {
    processTransfers(data.transfers);
    sinceId = data.nextSinceId;
  }

  if (data.hasMore) {
    // More data available, poll again immediately
    await poll();
  } else {
    // Wait before next poll
    setTimeout(poll, 5000);
  }
}
```

---

#### POST /erc20/batch/:chainId

Fetch transfers for multiple addresses in a single request.

**Parameters:**

| Name | Type | Location | Required | Description |
|------|------|----------|----------|-------------|
| chainId | number | path | Yes | Chain ID |

**Request Body:**

```json
{
  "addresses": [
    { "address": "0xabc...", "sinceId": 0 },
    { "address": "0xdef...", "sinceId": 1500 }
  ],
  "limit": 50,
  "direction": "both"
}
```

**Body Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| addresses | array | Yes | Array of address queries (max 500) |
| addresses[].address | string | Yes | Ethereum address |
| addresses[].sinceId | number | No | Cursor for this address (default: 0) |
| limit | number | No | Max results per address (default: 50, max: 100) |
| direction | string | No | `"from"`, `"to"`, or `"both"` (default: `"both"`) |

**Response:**

```json
{
  "success": true,
  "data": {
    "results": {
      "0xabc...": {
        "transfers": [...],
        "nextSinceId": 234,
        "hasMore": true
      },
      "0xdef...": {
        "transfers": [],
        "nextSinceId": 1500,
        "hasMore": false
      }
    },
    "timestamp": 1705000000
  }
}
```

**Batch Polling Pattern:**

```javascript
const cursors = {
  '0xabc...': 0,
  '0xdef...': 0,
  '0x123...': 0
};

async function batchPoll() {
  const addresses = Object.entries(cursors).map(([address, sinceId]) => ({
    address,
    sinceId
  }));

  const response = await fetch('https://erc20cache.bitxpay.com/erc20/batch/1', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ addresses, limit: 50 })
  });

  const { data } = await response.json();

  for (const [address, result] of Object.entries(data.results)) {
    if (result.transfers.length > 0) {
      processTransfers(address, result.transfers);
      cursors[address] = result.nextSinceId;
    }
  }

  setTimeout(batchPoll, 5000);
}
```

---

### Fusion+ Swap Endpoints

Fusion+ is 1inch's cross-chain atomic swap protocol.

#### GET /fusion-plus/swap/:orderHash

Get a specific Fusion+ swap by order hash.

**Response:**

```json
{
  "success": true,
  "data": {
    "id": 1,
    "order_hash": "0x...",
    "hashlock": "0x...",
    "secret": "0x...",
    "src_chain_id": 1,
    "src_tx_hash": "0x...",
    "src_block_number": 19000000,
    "src_block_timestamp": 1705000000,
    "src_maker": "0x...",
    "src_taker": "0x...",
    "src_token": "0x...",
    "src_amount": "1000000",
    "src_safety_deposit": "10000",
    "dst_chain_id": 137,
    "dst_tx_hash": "0x...",
    "dst_block_number": 52000000,
    "dst_block_timestamp": 1705000100,
    "dst_taker": "0x...",
    "withdrawal_tx_hash": "0x...",
    "withdrawal_block_number": 19000010,
    "status": "completed",
    "created_at": 1705000000
  }
}
```

**Swap Status Values:**

| Status | Description |
|--------|-------------|
| `src_escrow_created` | Source escrow created, waiting for destination |
| `dst_escrow_created` | Destination escrow created, ready for withdrawal |
| `completed` | Swap completed successfully |
| `cancelled` | Swap was cancelled |

---

#### GET /fusion-plus/address/:address

Get all Fusion+ swaps involving an address (as maker or taker).

---

#### GET /fusion-plus/pending

Get all pending (incomplete) Fusion+ swaps.

---

#### GET /fusion-plus/completed

Get all completed Fusion+ swaps.

---

#### GET /fusion-plus/src-chain/:chainId

Get Fusion+ swaps originating from a specific chain.

---

#### GET /fusion-plus/dst-chain/:chainId

Get Fusion+ swaps destined for a specific chain.

---

#### GET /transfer/swap/:chainId/:txHash

Get the Fusion+ swap associated with a specific transaction.

---

### Fusion Swap Endpoints

Fusion is 1inch's single-chain limit order protocol.

#### GET /fusion/swap/:orderHash

Get a specific Fusion swap by order hash.

**Response:**

```json
{
  "success": true,
  "data": {
    "id": 60,
    "order_hash": "0xe3e6cbc6...",
    "chain_id": 56,
    "tx_hash": "0x3c0bc2c7...",
    "block_number": 76912356,
    "block_timestamp": 1769155453,
    "log_index": 105,
    "maker": "0xf037087c...",
    "taker": "0xf037087c...",
    "maker_token": "0x93fac02b...",
    "taker_token": "0x55d39832...",
    "maker_amount": "0x1804f2dd1677ff38",
    "taker_amount": "0x2b162785a46603006",
    "remaining": "0x0",
    "is_partial_fill": 0,
    "status": "filled",
    "created_at": 1769155459
  }
}
```

**Fusion Status Values:**

| Status | Description |
|--------|-------------|
| `filled` | Order completely filled |
| `partially_filled` | Order partially filled |
| `cancelled` | Order was cancelled |

---

#### GET /fusion/maker/:address

Get Fusion swaps where address is the maker.

---

#### GET /fusion/taker/:address

Get Fusion swaps where address is the taker.

---

#### GET /fusion/chain/:chainId

Get all Fusion swaps on a specific chain.

---

#### GET /fusion/filled

Get all filled Fusion swaps.

---

#### GET /fusion/cancelled

Get all cancelled Fusion swaps.

---

#### GET /fusion/recent

Get most recent Fusion swaps across all chains.

---

### Crypto2Fiat Endpoints

Crypto2Fiat tracks fiat off-ramp transactions.

#### GET /crypto2fiat/order/:orderId

Get a Crypto2Fiat event by order ID.

**Response:**

```json
{
  "success": true,
  "data": {
    "id": 1,
    "order_id": "ORDER123",
    "chain_id": 1,
    "tx_hash": "0x...",
    "block_number": 19000000,
    "block_timestamp": 1705000000,
    "log_index": 0,
    "token": "0xdac17f958d2ee523a2206206994597c13d831ec7",
    "amount": "1000000",
    "recipient": "0x...",
    "created_at": 1705000000
  }
}
```

---

#### GET /crypto2fiat/recipient/:address

Get Crypto2Fiat events for a recipient address.

---

#### GET /crypto2fiat/chain/:chainId

Get Crypto2Fiat events on a specific chain.

---

#### GET /crypto2fiat/token/:token

Get Crypto2Fiat events for a specific token.

---

#### GET /crypto2fiat/recent

Get most recent Crypto2Fiat events.

---

## Data Types

### Transfer Object

```typescript
interface Transfer {
  chainId: number;          // Chain ID
  txHash: string;           // Transaction hash
  token: string;            // Token contract address
  from: string;             // Sender address
  to: string;               // Receiver address
  value: string;            // Transfer amount (hex or decimal string)
  blockNumber: number;      // Block number
  timestamp: number;        // Unix timestamp
  swapType: string | null;  // "fusion_plus", "fusion", "crypto_to_fiat", or null
}
```

### TransferWithId Object (Streaming endpoints)

```typescript
interface TransferWithId extends Transfer {
  id: number;  // Unique monotonic ID for cursor pagination
}
```

### StreamResult Object

```typescript
interface StreamResult {
  transfers: TransferWithId[];  // Array of transfers
  nextSinceId: number;          // Cursor for next request
  hasMore: boolean;             // More results available
}
```

### Swap Type Values

| Value | Description |
|-------|-------------|
| `null` | Regular ERC20 transfer |
| `"fusion_plus"` | Part of a Fusion+ cross-chain swap |
| `"fusion"` | Part of a Fusion single-chain swap |
| `"crypto_to_fiat"` | Crypto to fiat off-ramp transaction |

---

## Error Handling

### HTTP Status Codes

| Code | Description |
|------|-------------|
| 200 | Success |
| 400 | Bad Request (invalid parameters) |
| 404 | Endpoint not found |
| 500 | Internal server error |

### Common Errors

**Invalid address format:**
```json
{
  "success": false,
  "error": "Invalid address format: 0xinvalid"
}
```

**Batch limit exceeded:**
```json
{
  "success": false,
  "error": "Maximum 500 addresses allowed per request"
}
```

**Invalid JSON body:**
```json
{
  "success": false,
  "error": "Invalid JSON body"
}
```

**Invalid direction parameter:**
```json
{
  "success": false,
  "error": "direction must be \"from\", \"to\", or \"both\""
}
```

---

## Rate Limits

Currently no rate limits are enforced. Please use responsibly:

- Use batch endpoints instead of many individual requests
- Use `since_id` pagination to avoid re-fetching data
- Poll at reasonable intervals (5+ seconds for real-time use cases)

---

## Examples

### JavaScript/TypeScript

```typescript
// Single address streaming
async function streamTransfers(chainId: number, address: string) {
  let sinceId = 0;

  while (true) {
    const response = await fetch(
      `https://erc20cache.bitxpay.com/erc20/stream/${chainId}/${address}?since_id=${sinceId}&limit=100`
    );
    const { success, data, error } = await response.json();

    if (!success) {
      console.error('Error:', error);
      break;
    }

    for (const transfer of data.transfers) {
      console.log(`${transfer.from} -> ${transfer.to}: ${transfer.value}`);
      if (transfer.swapType) {
        console.log(`  Swap type: ${transfer.swapType}`);
      }
    }

    sinceId = data.nextSinceId;

    if (!data.hasMore) {
      await new Promise(resolve => setTimeout(resolve, 5000));
    }
  }
}

// Batch polling for multiple addresses
async function batchPoll(chainId: number, addresses: string[]) {
  const cursors: Record<string, number> = {};
  addresses.forEach(addr => cursors[addr] = 0);

  const response = await fetch(
    `https://erc20cache.bitxpay.com/erc20/batch/${chainId}`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        addresses: addresses.map(address => ({
          address,
          sinceId: cursors[address]
        })),
        limit: 50,
        direction: 'both'
      })
    }
  );

  const { data } = await response.json();

  for (const [address, result] of Object.entries(data.results)) {
    const { transfers, nextSinceId, hasMore } = result as any;
    console.log(`${address}: ${transfers.length} new transfers, hasMore: ${hasMore}`);
    cursors[address] = nextSinceId;
  }

  return cursors;
}
```

### Python

```python
import requests
import time

def stream_transfers(chain_id: int, address: str):
    since_id = 0
    base_url = "https://erc20cache.bitxpay.com"

    while True:
        response = requests.get(
            f"{base_url}/erc20/stream/{chain_id}/{address}",
            params={"since_id": since_id, "limit": 100}
        )
        data = response.json()

        if not data["success"]:
            print(f"Error: {data['error']}")
            break

        for transfer in data["data"]["transfers"]:
            print(f"{transfer['from']} -> {transfer['to']}: {transfer['value']}")
            if transfer["swapType"]:
                print(f"  Swap type: {transfer['swapType']}")

        since_id = data["data"]["nextSinceId"]

        if not data["data"]["hasMore"]:
            time.sleep(5)

def batch_poll(chain_id: int, addresses: list[str]):
    base_url = "https://erc20cache.bitxpay.com"

    response = requests.post(
        f"{base_url}/erc20/batch/{chain_id}",
        json={
            "addresses": [{"address": addr, "sinceId": 0} for addr in addresses],
            "limit": 50,
            "direction": "both"
        }
    )

    data = response.json()

    for address, result in data["data"]["results"].items():
        print(f"{address}: {len(result['transfers'])} transfers")

    return data["data"]["results"]
```

### cURL

```bash
# Get transfers for an address
curl "https://erc20cache.bitxpay.com/erc20/address/1/0x1234567890123456789012345678901234567890"

# Stream with since_id
curl "https://erc20cache.bitxpay.com/erc20/stream/1/0x1234567890123456789012345678901234567890?since_id=0&limit=10"

# Batch request
curl -X POST "https://erc20cache.bitxpay.com/erc20/batch/1" \
  -H "Content-Type: application/json" \
  -d '{
    "addresses": [
      {"address": "0xabc...", "sinceId": 0},
      {"address": "0xdef...", "sinceId": 100}
    ],
    "limit": 50,
    "direction": "both"
  }'

# Get Fusion swaps for a maker
curl "https://erc20cache.bitxpay.com/fusion/maker/0x1234567890123456789012345678901234567890"

# Get recent Fusion swaps
curl "https://erc20cache.bitxpay.com/fusion/recent"
```

---

## Changelog

### v2.0 (January 2026)

**New Features:**
- Added streaming endpoint with `since_id` cursor pagination
- Added batch endpoint for multi-address queries (up to 500 addresses)
- Expanded to 13 supported chains
- Per-chain database architecture for improved performance

**New Endpoints:**
- `GET /erc20/stream/:chainId/:address`
- `POST /erc20/batch/:chainId`

**Breaking Changes:**
- None. All v1.x endpoints remain compatible.

### v1.0 (Initial Release)

- ERC20 transfer indexing
- Fusion+ cross-chain swap support
- Fusion single-chain swap support
- Crypto2Fiat event tracking
