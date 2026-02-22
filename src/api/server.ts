import 'dotenv/config';
import http from 'http';
import fs from 'fs';
import path from 'path';
import WebSocket from 'ws';
import { PostgresCache } from '../cache/postgres';
import { QueryService } from '../services/queryService';
import { getNetworkConfig } from '../config/networks';

const databaseUrl = process.env.DATABASE_URL || 'postgres://erc20cache:erc20cache_pass@localhost:5433/erc20cache';
const cache = new PostgresCache(databaseUrl);
const queryService = new QueryService(cache);

interface APIResponse {
  success: boolean;
  data?: any;
  error?: string;
}

// Parse JSON body from POST request
async function parseJsonBody(req: http.IncomingMessage): Promise<any> {
  return new Promise((resolve, reject) => {
    let body = '';
    req.on('data', (chunk: Buffer) => body += chunk.toString());
    req.on('end', () => {
      try {
        resolve(body ? JSON.parse(body) : {});
      } catch (e) {
        reject(new Error('Invalid JSON body'));
      }
    });
    req.on('error', reject);
  });
}

// Validate batch request
function validateBatchRequest(body: any): { valid: boolean; error?: string } {
  if (!body.addresses || !Array.isArray(body.addresses)) {
    return { valid: false, error: 'addresses array is required' };
  }
  if (body.addresses.length > 500) {
    return { valid: false, error: 'Maximum 500 addresses allowed per request' };
  }
  if (body.addresses.length === 0) {
    return { valid: false, error: 'At least one address is required' };
  }

  const addressRegex = /^0x[a-fA-F0-9]{40}$/;
  for (const item of body.addresses) {
    if (!item.address || !addressRegex.test(item.address)) {
      return { valid: false, error: `Invalid address format: ${item.address}` };
    }
    if (item.sinceId !== undefined && (typeof item.sinceId !== 'number' || item.sinceId < 0)) {
      return { valid: false, error: `Invalid sinceId for ${item.address}` };
    }
  }

  if (body.limit !== undefined && (typeof body.limit !== 'number' || body.limit < 1 || body.limit > 100)) {
    return { valid: false, error: 'limit must be between 1 and 100' };
  }

  if (body.direction !== undefined && !['from', 'to', 'both'].includes(body.direction)) {
    return { valid: false, error: 'direction must be "from", "to", or "both"' };
  }

  return { valid: true };
}

async function handleRequest(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
  res.setHeader('Content-Type', 'application/json');
  res.setHeader('Access-Control-Allow-Origin', '*');

  const url = new URL(req.url || '/', `http://${req.headers.host}`);
  const path = url.pathname;

  try {
    // =========================================================================
    // Bitcoin Endpoints (chain_id = 0, non-EVM address formats)
    // =========================================================================

    // Bitcoin address pattern: base58 (1xxx, 3xxx) or bech32 (bc1xxx)
    const btcAddrPattern = /^(1[a-km-zA-HJ-NP-Z1-9]{25,34}|3[a-km-zA-HJ-NP-Z1-9]{25,34}|bc1[a-z0-9]{8,87})$/;

    // GET /btc/address/:address - transfers involving address (from or to)
    if (path.match(/^\/btc\/address\/[a-zA-Z0-9]{20,90}$/)) {
      const address = path.split('/')[3];
      if (!btcAddrPattern.test(address)) return sendResponse(res, 400, { success: false, error: 'Invalid Bitcoin address' });
      const transfers = await cache.getBitcoinTransfersByAddress(address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /btc/from/:address - transfers from address
    if (path.match(/^\/btc\/from\/[a-zA-Z0-9]{20,90}$/)) {
      const address = path.split('/')[3];
      if (!btcAddrPattern.test(address)) return sendResponse(res, 400, { success: false, error: 'Invalid Bitcoin address' });
      const transfers = await cache.getBitcoinTransfersByFrom(address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /btc/to/:address - transfers to address
    if (path.match(/^\/btc\/to\/[a-zA-Z0-9]{20,90}$/)) {
      const address = path.split('/')[3];
      if (!btcAddrPattern.test(address)) return sendResponse(res, 400, { success: false, error: 'Invalid Bitcoin address' });
      const transfers = await cache.getBitcoinTransfersByTo(address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /btc/tx/:txHash - all outputs for a Bitcoin transaction
    if (path.match(/^\/btc\/tx\/[a-fA-F0-9]{64}$/)) {
      const txHash = path.split('/')[3];
      const transfers = await cache.getBitcoinTransfersByTx(txHash);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /btc/pending - pending mempool transactions
    if (path === '/btc/pending') {
      const transfers = await cache.getBitcoinPendingTransfers();
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // =========================================================================
    // ERC20 Endpoints
    // =========================================================================

    // GET /erc20/from/:chainId/:address
    if (path.match(/^\/erc20\/from\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getERC20TransfersByFrom(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /erc20/to/:chainId/:address
    if (path.match(/^\/erc20\/to\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getERC20TransfersByTo(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /erc20/both/:chainId/:from/:to
    if (path.match(/^\/erc20\/both\/\d+\/0x[a-fA-F0-9]{40}\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, from, to] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getERC20TransfersByBoth(chainId, from, to);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /erc20/address/:chainId/:address
    if (path.match(/^\/erc20\/address\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getERC20TransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /native/from/:chainId/:address
    if (path.match(/^\/native\/from\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getNativeTransfersByFrom(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /native/to/:chainId/:address
    if (path.match(/^\/native\/to\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getNativeTransfersByTo(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /native/both/:chainId/:from/:to
    if (path.match(/^\/native\/both\/\d+\/0x[a-fA-F0-9]{40}\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, from, to] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getNativeTransfersByBoth(chainId, from, to);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /native/address/:chainId/:address
    if (path.match(/^\/native\/address\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getNativeTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /all/:chainId/:address
    if (path.match(/^\/all\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await queryService.getAllTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /networks
    if (path === '/networks') {
      const { SUPPORTED_NETWORKS } = await import('../config/networks');
      return sendResponse(res, 200, { success: true, data: SUPPORTED_NETWORKS });
    }

    // =========================================================================
    // Fusion+ Endpoints
    // =========================================================================

    // GET /fusion-plus/swap/:orderHash - Get swap by order hash
    if (path.match(/^\/fusion-plus\/swap\/0x[a-fA-F0-9]{64}$/)) {
      const orderHash = path.split('/')[3];
      const swap = await cache.getFusionPlusSwap(orderHash);
      if (swap) {
        return sendResponse(res, 200, { success: true, data: swap });
      }
      return sendResponse(res, 404, { success: false, error: 'Swap not found' });
    }

    // GET /fusion-plus/address/:address - Get swaps by maker/taker address
    if (path.match(/^\/fusion-plus\/address\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = await cache.getFusionPlusSwapsByAddress(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/pending - Get swaps awaiting dst escrow
    if (path === '/fusion-plus/pending') {
      const swaps = await cache.getFusionPlusPending();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/completed - Get fully completed swaps
    if (path === '/fusion-plus/completed') {
      const swaps = await cache.getFusionPlusCompleted();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/src-chain/:chainId - Get swaps by source chain
    if (path.match(/^\/fusion-plus\/src-chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = await cache.getFusionPlusSwapsBySrcChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/dst-chain/:chainId - Get swaps by destination chain
    if (path.match(/^\/fusion-plus\/dst-chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = await cache.getFusionPlusSwapsByDstChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /transfer/swap/:chainId/:txHash - Get swap details for a specific transfer
    if (path.match(/^\/transfer\/swap\/\d+\/0x[a-fA-F0-9]{64}$/)) {
      const [, , , chainIdStr, txHash] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const swap = await cache.getSwapForTransfer(chainId, txHash);
      return sendResponse(res, 200, { success: true, data: swap });
    }

    // GET /erc20/fusion-plus/:chainId/:address - Get fusion+ labeled transfers for an address
    if (path.match(/^\/erc20\/fusion-plus\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await cache.getFusionPlusTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /erc20/fusion/:chainId/:address - Get fusion labeled transfers for an address (backwards compat)
    if (path.match(/^\/erc20\/fusion\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await cache.getFusionPlusTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // =========================================================================
    // Fusion (Single-Chain) Endpoints
    // =========================================================================

    // GET /fusion/swap/:orderHash - Get swap by order hash
    if (path.match(/^\/fusion\/swap\/0x[a-fA-F0-9]{64}$/)) {
      const orderHash = path.split('/')[3];
      const swap = await cache.getFusionSwap(orderHash);
      if (swap) {
        return sendResponse(res, 200, { success: true, data: swap });
      }
      return sendResponse(res, 404, { success: false, error: 'Swap not found' });
    }

    // GET /fusion/maker/:address - Get swaps by maker address
    if (path.match(/^\/fusion\/maker\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = await cache.getFusionSwapsByMaker(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/taker/:address - Get swaps by taker address (recipient of output tokens)
    if (path.match(/^\/fusion\/taker\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = await cache.getFusionSwapsByTaker(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/chain/:chainId - Get swaps by chain
    if (path.match(/^\/fusion\/chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = await cache.getFusionSwapsByChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/filled - Get filled swaps
    if (path === '/fusion/filled') {
      const swaps = await cache.getFusionSwapsByStatus('filled');
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/cancelled - Get cancelled swaps
    if (path === '/fusion/cancelled') {
      const swaps = await cache.getFusionSwapsByStatus('cancelled');
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/recent - Get recent swaps
    if (path === '/fusion/recent') {
      const swaps = await cache.getRecentFusionSwaps();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /erc20/fusion-single/:chainId/:address - Get fusion-labeled transfers (single-chain)
    if (path.match(/^\/erc20\/fusion-single\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await cache.getFusionTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /stats - Get database statistics
    if (path === '/stats') {
      const stats = await cache.getStats();
      return sendResponse(res, 200, { success: true, data: stats });
    }

    // =========================================================================
    // Crypto2Fiat Endpoints
    // =========================================================================

    // GET /crypto2fiat/order/:orderId - Get C2F event by order ID
    if (path.match(/^\/crypto2fiat\/order\/0x[a-fA-F0-9]{64}$/)) {
      const orderId = path.split('/')[3];
      const event = await cache.getCrypto2FiatByOrderId(orderId);
      if (event) {
        return sendResponse(res, 200, { success: true, data: event });
      }
      return sendResponse(res, 404, { success: false, error: 'Crypto2Fiat event not found' });
    }

    // GET /crypto2fiat/recipient/:address - Get C2F events by recipient
    if (path.match(/^\/crypto2fiat\/recipient\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const events = await cache.getCrypto2FiatByRecipient(address);
      return sendResponse(res, 200, { success: true, data: events });
    }

    // GET /crypto2fiat/chain/:chainId - Get C2F events by chain
    if (path.match(/^\/crypto2fiat\/chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const events = await cache.getCrypto2FiatByChain(chainId);
      return sendResponse(res, 200, { success: true, data: events });
    }

    // GET /crypto2fiat/token/:token - Get C2F events by token
    if (path.match(/^\/crypto2fiat\/token\/0x[a-fA-F0-9]{40}$/)) {
      const token = path.split('/')[3];
      const events = await cache.getCrypto2FiatByToken(token);
      return sendResponse(res, 200, { success: true, data: events });
    }

    // GET /crypto2fiat/recent - Get recent C2F events
    if (path === '/crypto2fiat/recent') {
      const events = await cache.getRecentCrypto2FiatEvents();
      return sendResponse(res, 200, { success: true, data: events });
    }

    // GET /crypto2fiat/tx/:chainId/:txHash - Get C2F event by transaction hash
    if (path.match(/^\/crypto2fiat\/tx\/\d+\/0x[a-fA-F0-9]{64}$/)) {
      const [, , , chainIdStr, txHash] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const event = await cache.getCrypto2FiatByTxHash(chainId, txHash);
      if (event) {
        return sendResponse(res, 200, { success: true, data: event });
      }
      return sendResponse(res, 404, { success: false, error: 'Crypto2Fiat event not found' });
    }

    // GET /erc20/crypto2fiat/:chainId/:address - Get C2F labeled transfers for an address
    if (path.match(/^\/erc20\/crypto2fiat\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = await cache.getCrypto2FiatTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // =========================================================================
    // Streaming/Batch Endpoints (since_id pagination)
    // =========================================================================

    // OPTIONS - CORS preflight
    if (req.method === 'OPTIONS') {
      res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
      res.setHeader('Access-Control-Allow-Headers', 'Content-Type, X-API-Key');
      res.statusCode = 204;
      res.end();
      return;
    }

    // GET /erc20/stream/:chainId/:address - Stream transfers with since_id
    if (req.method === 'GET' && path.match(/^\/erc20\/stream\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);

      const sinceId = parseInt(url.searchParams.get('since_id') || '0') || 0;
      const limit = Math.min(parseInt(url.searchParams.get('limit') || '100') || 100, 1000);
      const direction = (url.searchParams.get('direction') || 'both') as 'from' | 'to' | 'both';

      const result = await cache.getERC20TransfersStream(chainId, address, { sinceId, limit, direction });
      return sendResponse(res, 200, { success: true, data: result });
    }

    // POST /erc20/batch/:chainId - Batch address fetching
    if (req.method === 'POST' && path.match(/^\/erc20\/batch\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);

      try {
        const body = await parseJsonBody(req);
        const validation = validateBatchRequest(body);

        if (!validation.valid) {
          return sendResponse(res, 400, { success: false, error: validation.error });
        }

        const queries = body.addresses.map((item: any) => ({
          address: item.address,
          sinceId: item.sinceId || 0
        }));

        const results = await cache.getERC20TransfersBatch(
          chainId,
          queries,
          body.limit || 50,
          body.direction || 'both'
        );

        return sendResponse(res, 200, {
          success: true,
          data: { results, timestamp: Math.floor(Date.now() / 1000) }
        });
      } catch (error: any) {
        return sendResponse(res, 400, { success: false, error: error.message });
      }
    }

    // =========================================================================
    // Admin API (API key protected — runtime config tuning)
    // =========================================================================

    if (path.startsWith('/admin/')) {
      const adminApiKey = process.env.ADMIN_API_KEY || '';
      const requestKey = req.headers['x-api-key'] || '';
      if (!adminApiKey || requestKey !== adminApiKey) {
        return sendResponse(res, 401, { success: false, error: 'Unauthorized: invalid or missing X-API-Key header' });
      }

      // Rust-side default configs (must match config.rs)
      const CHAIN_DEFAULTS: Record<number, { name: string, blocks_per_request: number, concurrent_fetches: number, poll_interval_ms: number, confirmation_blocks: number, copy_threshold: number, concurrent_inserts: number }> = {
        0:     { name: 'Bitcoin',        blocks_per_request: 1,   concurrent_fetches: 2,  poll_interval_ms: 5000, confirmation_blocks: 6,  copy_threshold: 1, concurrent_inserts: 3 },
        1:     { name: 'Ethereum',       blocks_per_request: 10,  concurrent_fetches: 10, poll_interval_ms: 500,  confirmation_blocks: 12, copy_threshold: 1, concurrent_inserts: 3 },
        8453:  { name: 'Base',            blocks_per_request: 10,  concurrent_fetches: 15, poll_interval_ms: 300,  confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        42161: { name: 'Arbitrum One',    blocks_per_request: 50,  concurrent_fetches: 15, poll_interval_ms: 100,  confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        137:   { name: 'Polygon',         blocks_per_request: 10,  concurrent_fetches: 15, poll_interval_ms: 300,  confirmation_blocks: 50, copy_threshold: 1, concurrent_inserts: 3 },
        56:    { name: 'BNB Smart Chain', blocks_per_request: 30,  concurrent_fetches: 15, poll_interval_ms: 500,  confirmation_blocks: 3,  copy_threshold: 1, concurrent_inserts: 3 },
        10:    { name: 'OP Mainnet',      blocks_per_request: 50,  concurrent_fetches: 5,  poll_interval_ms: 500,  confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        43114: { name: 'Avalanche',       blocks_per_request: 50,  concurrent_fetches: 5,  poll_interval_ms: 500,  confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        100:   { name: 'Gnosis',          blocks_per_request: 50,  concurrent_fetches: 5,  poll_interval_ms: 500,  confirmation_blocks: 12, copy_threshold: 1, concurrent_inserts: 3 },
        1868:  { name: 'Soneium',         blocks_per_request: 50,  concurrent_fetches: 5,  poll_interval_ms: 500,  confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        59144: { name: 'Linea',           blocks_per_request: 200, concurrent_fetches: 3,  poll_interval_ms: 1000, confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        130:   { name: 'Unichain',        blocks_per_request: 200, concurrent_fetches: 3,  poll_interval_ms: 1000, confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        146:   { name: 'Sonic',           blocks_per_request: 200, concurrent_fetches: 3,  poll_interval_ms: 1000, confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
        57073: { name: 'Ink',             blocks_per_request: 200, concurrent_fetches: 3,  poll_interval_ms: 1000, confirmation_blocks: 1,  copy_threshold: 1, concurrent_inserts: 3 },
      };

      // GET /admin/config — list all chains with defaults + overrides
      if (req.method === 'GET' && path === '/admin/config') {
        const overrides = await cache.getConfigOverrides();
        const overrideMap: Record<number, any> = {};
        for (const ov of overrides) overrideMap[ov.chain_id] = ov;

        const configs = Object.entries(CHAIN_DEFAULTS).map(([chainIdStr, defaults]) => {
          const chainId = parseInt(chainIdStr);
          const ov = overrideMap[chainId];
          return {
            chain_id: chainId,
            name: defaults.name,
            defaults,
            overrides: ov || null,
            effective: {
              blocks_per_request: ov?.blocks_per_request ?? defaults.blocks_per_request,
              concurrent_fetches: ov?.concurrent_fetches ?? defaults.concurrent_fetches,
              poll_interval_ms: ov?.poll_interval_ms ?? defaults.poll_interval_ms,
              confirmation_blocks: ov?.confirmation_blocks ?? defaults.confirmation_blocks,
              copy_threshold: ov?.copy_threshold ?? defaults.copy_threshold,
              concurrent_inserts: ov?.concurrent_inserts ?? defaults.concurrent_inserts,
            }
          };
        });
        return sendResponse(res, 200, { success: true, data: configs });
      }

      // GET /admin/config/:chainId
      if (req.method === 'GET' && path.match(/^\/admin\/config\/\d+$/)) {
        const chainId = parseInt(path.split('/')[3]);
        const defaults = CHAIN_DEFAULTS[chainId];
        if (!defaults) return sendResponse(res, 404, { success: false, error: `Unknown chain_id: ${chainId}` });

        const override = await cache.getConfigOverride(chainId);
        return sendResponse(res, 200, {
          success: true,
          data: {
            chain_id: chainId,
            name: defaults.name,
            defaults,
            overrides: override || null,
            effective: {
              blocks_per_request: override?.blocks_per_request ?? defaults.blocks_per_request,
              concurrent_fetches: override?.concurrent_fetches ?? defaults.concurrent_fetches,
              poll_interval_ms: override?.poll_interval_ms ?? defaults.poll_interval_ms,
              confirmation_blocks: override?.confirmation_blocks ?? defaults.confirmation_blocks,
              copy_threshold: override?.copy_threshold ?? defaults.copy_threshold,
              concurrent_inserts: override?.concurrent_inserts ?? defaults.concurrent_inserts,
            }
          }
        });
      }

      // PUT /admin/config/:chainId — set override
      if (req.method === 'PUT' && path.match(/^\/admin\/config\/\d+$/)) {
        const chainId = parseInt(path.split('/')[3]);
        const defaults = CHAIN_DEFAULTS[chainId];
        if (!defaults) return sendResponse(res, 404, { success: false, error: `Unknown chain_id: ${chainId}` });

        try {
          const body = await parseJsonBody(req);

          // Validate bounds
          if (body.blocks_per_request !== undefined) {
            const v = parseInt(body.blocks_per_request);
            if (isNaN(v) || v < 1 || v > 1000) return sendResponse(res, 400, { success: false, error: 'blocks_per_request must be 1-1000' });
            body.blocks_per_request = v;
          }
          if (body.concurrent_fetches !== undefined) {
            const v = parseInt(body.concurrent_fetches);
            if (isNaN(v) || v < 1 || v > 50) return sendResponse(res, 400, { success: false, error: 'concurrent_fetches must be 1-50' });
            body.concurrent_fetches = v;
          }
          if (body.poll_interval_ms !== undefined) {
            const v = parseInt(body.poll_interval_ms);
            if (isNaN(v) || v < 50 || v > 10000) return sendResponse(res, 400, { success: false, error: 'poll_interval_ms must be 50-10000' });
            body.poll_interval_ms = v;
          }
          if (body.confirmation_blocks !== undefined) {
            const v = parseInt(body.confirmation_blocks);
            if (isNaN(v) || v < 0 || v > 200) return sendResponse(res, 400, { success: false, error: 'confirmation_blocks must be 0-200' });
            body.confirmation_blocks = v;
          }
          if (body.copy_threshold !== undefined) {
            const v = parseInt(body.copy_threshold);
            if (isNaN(v) || v < 1 || v > 100000) return sendResponse(res, 400, { success: false, error: 'copy_threshold must be 1-100000' });
            body.copy_threshold = v;
          }
          if (body.concurrent_inserts !== undefined) {
            const v = parseInt(body.concurrent_inserts);
            if (isNaN(v) || v < 1 || v > 10) return sendResponse(res, 400, { success: false, error: 'concurrent_inserts must be 1-10' });
            body.concurrent_inserts = v;
          }

          await cache.upsertConfigOverride(chainId, body);
          const updated = await cache.getConfigOverride(chainId);
          return sendResponse(res, 200, {
            success: true,
            data: {
              chain_id: chainId,
              name: defaults.name,
              overrides: updated,
              effective: {
                blocks_per_request: updated?.blocks_per_request ?? defaults.blocks_per_request,
                concurrent_fetches: updated?.concurrent_fetches ?? defaults.concurrent_fetches,
                poll_interval_ms: updated?.poll_interval_ms ?? defaults.poll_interval_ms,
                confirmation_blocks: updated?.confirmation_blocks ?? defaults.confirmation_blocks,
                copy_threshold: updated?.copy_threshold ?? defaults.copy_threshold,
                concurrent_inserts: updated?.concurrent_inserts ?? defaults.concurrent_inserts,
              },
              note: 'Config watcher picks up changes within 5 seconds'
            }
          });
        } catch (error: any) {
          return sendResponse(res, 400, { success: false, error: error.message });
        }
      }

      // DELETE /admin/config/:chainId — remove override (revert to defaults)
      if (req.method === 'DELETE' && path.match(/^\/admin\/config\/\d+$/)) {
        const chainId = parseInt(path.split('/')[3]);
        const defaults = CHAIN_DEFAULTS[chainId];
        if (!defaults) return sendResponse(res, 404, { success: false, error: `Unknown chain_id: ${chainId}` });

        const deleted = await cache.deleteConfigOverride(chainId);
        return sendResponse(res, 200, {
          success: true,
          data: {
            chain_id: chainId,
            name: defaults.name,
            reverted_to_defaults: deleted,
            effective: defaults,
            note: 'Config watcher will revert to defaults within 5 seconds'
          }
        });
      }

      return sendResponse(res, 404, { success: false, error: 'Admin endpoint not found' });
    }

    // =========================================================================
    // Monitoring Dashboard
    // =========================================================================

    // GET /monitor - Serve monitoring dashboard HTML
    if (path === '/monitor') {
      try {
        const filePath = __filename.replace(/server\.[jt]s$/, 'monitor.html');
        const html = fs.readFileSync(filePath, 'utf-8');
        res.setHeader('Content-Type', 'text/html');
        res.statusCode = 200;
        res.end(html);
        return;
      } catch (err: any) {
        return sendResponse(res, 500, { success: false, error: 'Failed to load monitor page: ' + err.message });
      }
    }

    // GET /monitor/stats - JSON endpoint for listener stats
    if (path === '/monitor/stats') {
      const stats = await cache.getListenerStats();
      return sendResponse(res, 200, { success: true, data: stats });
    }

    // GET /monitor/history?chain_id=137&minutes=60 - Historical metrics for charts
    if (path === '/monitor/history') {
      const chainId = parseInt(url.searchParams.get('chain_id') || '0');
      const minutes = parseInt(url.searchParams.get('minutes') || '60') || 60;
      if (isNaN(chainId)) return sendResponse(res, 400, { success: false, error: 'chain_id query parameter is required' });
      const history = await cache.getMetricsHistory(chainId, Math.min(minutes, 60));
      return sendResponse(res, 200, { success: true, data: history });
    }

    // 404 Not Found
    return sendResponse(res, 404, { success: false, error: 'Endpoint not found' });
  } catch (error: any) {
    console.error('API Error:', error);
    return sendResponse(res, 500, { success: false, error: error.message });
  }
}

function sendResponse(res: http.ServerResponse, statusCode: number, data: APIResponse): void {
  res.statusCode = statusCode;
  res.end(JSON.stringify(data, null, 2));
}

async function startServer(): Promise<void> {
  // Test PostgreSQL connection
  const healthy = await cache.isHealthy();
  if (!healthy) {
    console.error('Failed to connect to PostgreSQL');
    process.exit(1);
  }
  console.log(`PostgreSQL connected: ${databaseUrl.replace(/:[^:@]+@/, ':****@')}`);

  const PORT = process.env.API_PORT || 3000;
  const server = http.createServer(handleRequest);

  server.listen(PORT, () => {
    console.log(`API Server running on http://localhost:${PORT}`);
    console.log('\nAvailable endpoints:');
    console.log('  GET /networks');
    console.log('  GET /stats');
    console.log('\n  Bitcoin:');
    console.log('  GET /btc/address/:address');
    console.log('  GET /btc/from/:address');
    console.log('  GET /btc/to/:address');
    console.log('  GET /btc/tx/:txHash');
    console.log('  GET /btc/pending');
    console.log('\n  ERC20 Transfers:');
    console.log('  GET /erc20/from/:chainId/:address');
    console.log('  GET /erc20/to/:chainId/:address');
    console.log('  GET /erc20/both/:chainId/:from/:to');
    console.log('  GET /erc20/address/:chainId/:address');
    console.log('  GET /erc20/fusion-plus/:chainId/:address');
    console.log('  GET /erc20/fusion-single/:chainId/:address');
    console.log('\n  Native Transfers (not supported):');
    console.log('  GET /native/from/:chainId/:address');
    console.log('  GET /native/to/:chainId/:address');
    console.log('  GET /native/both/:chainId/:from/:to');
    console.log('  GET /native/address/:chainId/:address');
    console.log('\n  Combined:');
    console.log('  GET /all/:chainId/:address');
    console.log('\n  Fusion+ Swaps (Cross-Chain):');
    console.log('  GET /fusion-plus/swap/:orderHash');
    console.log('  GET /fusion-plus/address/:address');
    console.log('  GET /fusion-plus/pending');
    console.log('  GET /fusion-plus/completed');
    console.log('  GET /fusion-plus/src-chain/:chainId');
    console.log('  GET /fusion-plus/dst-chain/:chainId');
    console.log('  GET /transfer/swap/:chainId/:txHash');
    console.log('\n  Fusion Swaps (Single-Chain):');
    console.log('  GET /fusion/swap/:orderHash');
    console.log('  GET /fusion/maker/:address');
    console.log('  GET /fusion/taker/:address');
    console.log('  GET /fusion/chain/:chainId');
    console.log('  GET /fusion/filled');
    console.log('  GET /fusion/cancelled');
    console.log('  GET /fusion/recent');
    console.log('\n  Crypto2Fiat Events:');
    console.log('  GET /crypto2fiat/order/:orderId');
    console.log('  GET /crypto2fiat/recipient/:address');
    console.log('  GET /crypto2fiat/chain/:chainId');
    console.log('  GET /crypto2fiat/token/:token');
    console.log('  GET /crypto2fiat/recent');
    console.log('  GET /crypto2fiat/tx/:chainId/:txHash');
    console.log('  GET /erc20/crypto2fiat/:chainId/:address');
    console.log('\n  Streaming/Batch (since_id pagination):');
    console.log('  GET  /erc20/stream/:chainId/:address?since_id=X&limit=Y&direction=both');
    console.log('  POST /erc20/batch/:chainId  (body: {addresses: [{address, sinceId}], limit, direction})');
  });

  // WebSocket server for live monitoring stats
  const wss = new WebSocket.Server({ noServer: true });

  server.on('upgrade', (req: http.IncomingMessage, socket: any, head: Buffer) => {
    const { pathname } = new URL(req.url || '/', `http://${req.headers.host}`);
    if (pathname === '/monitor/ws') {
      wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws));
    } else {
      socket.destroy();
    }
  });

  // Broadcast stats every 1 second (only when clients connected)
  const statsInterval = setInterval(async () => {
    if (wss.clients.size === 0) return;
    try {
      const stats = await cache.getListenerStats();
      const msg = JSON.stringify({ type: 'stats', timestamp: Date.now(), chains: stats });
      wss.clients.forEach((c) => {
        if (c.readyState === WebSocket.OPEN) c.send(msg);
      });
    } catch (err) {
      // ignore broadcast errors
    }
  }, 1000);

  console.log('  WebSocket: ws://localhost:' + PORT + '/monitor/ws');
  console.log('  Dashboard: http://localhost:' + PORT + '/monitor');

  // Graceful shutdown
  process.on('SIGINT', async () => {
    console.log('\nShutting down API server...');
    clearInterval(statsInterval);
    wss.close();
    server.close();
    await cache.close();
    console.log('Shutdown complete');
    process.exit(0);
  });
}

startServer().catch((error) => {
  console.error('Failed to start API server:', error);
  process.exit(1);
});
