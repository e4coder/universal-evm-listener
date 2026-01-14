# ERC20 Cache API - Test Documentation

Base URL: `https://erc20cache.bitxpay.com`

## Table of Contents

1. [General Endpoints](#general-endpoints)
2. [ERC20 Transfer Endpoints](#erc20-transfer-endpoints)
3. [Fusion+ Swap Endpoints](#fusion-swap-endpoints)
4. [Test Results Summary](#test-results-summary)

---

## General Endpoints

### GET /stats
Returns database statistics including transfer and Fusion+ swap counts.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/stats
```

**Response:**
```json
{
  "success": true,
  "data": {
    "transferCount": 764678,
    "fusionPlusCount": 1
  }
}
```

---

### GET /networks
Returns list of supported blockchain networks.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/networks
```

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
    {
      "name": "Arbitrum One",
      "chainId": 42161,
      "alchemyNetwork": "arb-mainnet",
      "nativeSymbol": "ETH"
    },
    {
      "name": "Polygon",
      "chainId": 137,
      "alchemyNetwork": "polygon-mainnet",
      "nativeSymbol": "MATIC"
    }
    // ... more networks
  ]
}
```

---

## ERC20 Transfer Endpoints

### GET /erc20/from/:chainId/:address
Get ERC20 transfers sent FROM an address.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/erc20/from/1/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "chainId": 1,
      "txHash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1",
      "token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "from": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "to": "0x101cddf458b11bfbb5e7b95323e3804e4a6b942e",
      "value": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "blockNumber": 24234081,
      "timestamp": 1768407431
    }
  ]
}
```

---

### GET /erc20/to/:chainId/:address
Get ERC20 transfers sent TO an address.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/erc20/to/1/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "chainId": 1,
      "txHash": "0x6e5ac8eec401bead87cbc5f6b139a542ab927a35a3d61dbede2ac0adb643aa0a",
      "token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "from": "0x2a7539f4bb5ddd12511ffb1bc15bcc29214492e2",
      "to": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "value": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "blockNumber": 24234077,
      "timestamp": 1768407383
    }
  ]
}
```

---

### GET /erc20/both/:chainId/:from/:to
Get ERC20 transfers between two specific addresses.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/erc20/both/1/0xFromAddress/0xToAddress
```

**Response:**
```json
{
  "success": true,
  "data": []
}
```

---

### GET /erc20/address/:chainId/:address
Get all ERC20 transfers involving an address (sent or received).

**Request:**
```bash
curl https://erc20cache.bitxpay.com/erc20/address/1/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "chainId": 1,
      "txHash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1",
      "token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "from": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "to": "0x101cddf458b11bfbb5e7b95323e3804e4a6b942e",
      "value": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "blockNumber": 24234081,
      "timestamp": 1768407431
    },
    {
      "chainId": 1,
      "txHash": "0x6e5ac8eec401bead87cbc5f6b139a542ab927a35a3d61dbede2ac0adb643aa0a",
      "token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "from": "0x2a7539f4bb5ddd12511ffb1bc15bcc29214492e2",
      "to": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "value": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "blockNumber": 24234077,
      "timestamp": 1768407383
    }
  ]
}
```

---

### GET /erc20/fusion/:chainId/:address
Get Fusion+ labeled transfers for an address.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/erc20/fusion/1/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "chainId": 1,
      "txHash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1",
      "token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "from": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "to": "0x101cddf458b11bfbb5e7b95323e3804e4a6b942e",
      "value": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "blockNumber": 24234081,
      "timestamp": 1768407431,
      "swapType": "fusion_plus"
    }
  ]
}
```

---

### GET /all/:chainId/:address
Get all transfers (ERC20 + native) for an address.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/all/1/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": {
    "erc20": [...],
    "native": []
  }
}
```

---

## Fusion+ Swap Endpoints

### GET /fusion-plus/pending
Get Fusion+ swaps awaiting destination escrow creation.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/pending
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": 1,
      "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
      "hashlock": "0x0ee10c7b2211b6793c943178c7ac762ed0e254bd73bd4265b83a09a4ca87ceb2",
      "secret": null,
      "src_chain_id": 1,
      "src_tx_hash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1",
      "src_block_number": 24234081,
      "src_block_timestamp": 1768407431,
      "src_log_index": 243,
      "src_escrow_address": null,
      "src_maker": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "src_taker": "0x33b41fe18d3a39046ad672f8a0c8c415454f629c",
      "src_token": "0x455e53cbb86018ac2b8092fdcd39d8444affc3f6",
      "src_amount": "0x00000000000000000000000000000000000000000000001efe1890f7e959ffd1",
      "src_safety_deposit": "0x000000000000000000000000000000000000000000000000000045276639b8e0",
      "src_timelocks": "0x6967c1870000018a000001120000000a00000256000001de0000012a00000018",
      "src_status": "created",
      "dst_chain_id": 137,
      "dst_tx_hash": null,
      "dst_block_number": null,
      "dst_block_timestamp": null,
      "dst_log_index": null,
      "dst_escrow_address": null,
      "dst_maker": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "dst_taker": null,
      "dst_token": "0xc2132d05d31c914a87c6611c10748aeb04b58e8f",
      "dst_amount": "0x00000000000000000000000000000000000000000000000000000000055bd1dd",
      "dst_safety_deposit": "0x000000000000000000000000000000000000000000000000021fd2986f3d8190",
      "dst_timelocks": null,
      "dst_status": "pending",
      "created_at": 1768407474,
      "updated_at": 1768407474
    }
  ]
}
```

---

### GET /fusion-plus/completed
Get fully completed Fusion+ swaps (both sides withdrawn).

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/completed
```

**Response:**
```json
{
  "success": true,
  "data": []
}
```

---

### GET /fusion-plus/swap/:orderHash
Get a specific Fusion+ swap by order hash.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/swap/0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe
```

**Response:**
```json
{
  "success": true,
  "data": {
    "id": 1,
    "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
    "hashlock": "0x0ee10c7b2211b6793c943178c7ac762ed0e254bd73bd4265b83a09a4ca87ceb2",
    "secret": null,
    "src_chain_id": 1,
    "src_tx_hash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1",
    "src_maker": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
    "src_status": "created",
    "dst_chain_id": 137,
    "dst_status": "pending"
    // ... full swap data
  }
}
```

**Error Response (404):**
```json
{
  "success": false,
  "error": "Swap not found"
}
```

---

### GET /fusion-plus/address/:address
Get all Fusion+ swaps involving an address (as maker or taker).

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/address/0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": 1,
      "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
      "src_maker": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5",
      "dst_maker": "0x335dc7abe02d1e1a51043d553349ea3b8e5f24c5"
      // ... full swap data
    }
  ]
}
```

---

### GET /fusion-plus/src-chain/:chainId
Get Fusion+ swaps originating from a specific chain.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/src-chain/1
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": 1,
      "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
      "src_chain_id": 1,
      "dst_chain_id": 137
      // ... full swap data
    }
  ]
}
```

---

### GET /fusion-plus/dst-chain/:chainId
Get Fusion+ swaps destined for a specific chain.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/fusion-plus/dst-chain/137
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": 1,
      "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
      "src_chain_id": 1,
      "dst_chain_id": 137
      // ... full swap data
    }
  ]
}
```

---

### GET /transfer/swap/:chainId/:txHash
Get Fusion+ swap details for a specific transfer transaction.

**Request:**
```bash
curl https://erc20cache.bitxpay.com/transfer/swap/1/0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1
```

**Response:**
```json
{
  "success": true,
  "data": {
    "id": 1,
    "order_hash": "0x3a0fe2bca3d3d92c35c26101a5e8335a147c5bb2d4d974fb27d5e8476914fafe",
    "src_tx_hash": "0xddbc8fa4a7ff6d71e4807524139af9b19c314ddf2cf690d2163b7f57a063a1c1"
    // ... full swap data
  }
}
```

---

## Test Results Summary

| Endpoint | Status | Notes |
|----------|--------|-------|
| `GET /stats` | ✅ PASS | Returns 764,678 transfers, 1 Fusion+ swap |
| `GET /networks` | ✅ PASS | Returns 13 supported chains |
| `GET /erc20/from/:chainId/:address` | ✅ PASS | Returns transfers sent from address |
| `GET /erc20/to/:chainId/:address` | ✅ PASS | Returns transfers received by address |
| `GET /erc20/both/:chainId/:from/:to` | ✅ PASS | Returns transfers between addresses |
| `GET /erc20/address/:chainId/:address` | ✅ PASS | Returns all transfers for address |
| `GET /erc20/fusion/:chainId/:address` | ✅ PASS | Returns fusion-labeled transfers with swapType |
| `GET /all/:chainId/:address` | ✅ PASS | Returns combined ERC20 and native |
| `GET /fusion-plus/pending` | ✅ PASS | Returns pending cross-chain swaps |
| `GET /fusion-plus/completed` | ✅ PASS | Returns empty (no completed in cache) |
| `GET /fusion-plus/swap/:orderHash` | ✅ PASS | Returns swap by order hash |
| `GET /fusion-plus/address/:address` | ✅ PASS | Returns swaps for address |
| `GET /fusion-plus/src-chain/:chainId` | ✅ PASS | Returns swaps by source chain |
| `GET /fusion-plus/dst-chain/:chainId` | ✅ PASS | Returns swaps by destination chain |
| `GET /transfer/swap/:chainId/:txHash` | ✅ PASS | Returns swap for transfer tx |

---

## Supported Chains

| Chain | Chain ID |
|-------|----------|
| Ethereum | 1 |
| Arbitrum One | 42161 |
| Polygon | 137 |
| OP Mainnet | 10 |
| Base | 8453 |
| Gnosis | 100 |
| BNB Smart Chain | 56 |
| Avalanche | 43114 |
| Scroll | 534352 |
| ZKsync Era | 324 |
| Linea | 59144 |
| Blast | 81457 |
| Sonic | 146 |

---

## Notes

1. **TTL**: Data is cached for 10 minutes (600 seconds) by default
2. **Fusion+ Swaps**: Cross-chain atomic swaps via 1inch Fusion+ protocol
3. **swap_type field**: Transfers involved in Fusion+ swaps are labeled with `swapType: "fusion_plus"`
4. **Nullable fields**: Destination chain fields are null until `DstEscrowCreated` event is captured
5. **Case insensitivity**: All addresses are normalized to lowercase internally
