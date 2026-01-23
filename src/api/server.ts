import 'dotenv/config';
import http from 'http';
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

    // OPTIONS - CORS preflight for POST requests
    if (req.method === 'OPTIONS') {
      res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
      res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
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
    console.log('  GET /erc20/crypto2fiat/:chainId/:address');
    console.log('\n  Streaming/Batch (since_id pagination):');
    console.log('  GET  /erc20/stream/:chainId/:address?since_id=X&limit=Y&direction=both');
    console.log('  POST /erc20/batch/:chainId  (body: {addresses: [{address, sinceId}], limit, direction})');
  });

  // Graceful shutdown
  process.on('SIGINT', async () => {
    console.log('\nShutting down API server...');
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
