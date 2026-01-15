import 'dotenv/config';
import http from 'http';
import { SQLiteCache } from '../cache/sqlite';
import { QueryService } from '../services/queryService';
import { getNetworkConfig } from '../config/networks';

const sqlitePath = process.env.SQLITE_PATH || '/home/ubuntu/universal_listener/data/transfers.db';
const cache = new SQLiteCache(sqlitePath);
const queryService = new QueryService(cache);

interface APIResponse {
  success: boolean;
  data?: any;
  error?: string;
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
      const swap = cache.getFusionPlusSwap(orderHash);
      if (swap) {
        return sendResponse(res, 200, { success: true, data: swap });
      }
      return sendResponse(res, 404, { success: false, error: 'Swap not found' });
    }

    // GET /fusion-plus/address/:address - Get swaps by maker/taker address
    if (path.match(/^\/fusion-plus\/address\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = cache.getFusionPlusSwapsByAddress(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/pending - Get swaps awaiting dst escrow
    if (path === '/fusion-plus/pending') {
      const swaps = cache.getFusionPlusPending();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/completed - Get fully completed swaps
    if (path === '/fusion-plus/completed') {
      const swaps = cache.getFusionPlusCompleted();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/src-chain/:chainId - Get swaps by source chain
    if (path.match(/^\/fusion-plus\/src-chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = cache.getFusionPlusSwapsBySrcChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion-plus/dst-chain/:chainId - Get swaps by destination chain
    if (path.match(/^\/fusion-plus\/dst-chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = cache.getFusionPlusSwapsByDstChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /transfer/swap/:chainId/:txHash - Get swap details for a specific transfer
    if (path.match(/^\/transfer\/swap\/\d+\/0x[a-fA-F0-9]{64}$/)) {
      const [, , , chainIdStr, txHash] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const swap = cache.getSwapForTransfer(chainId, txHash);
      return sendResponse(res, 200, { success: true, data: swap });
    }

    // GET /erc20/fusion-plus/:chainId/:address - Get fusion+ labeled transfers for an address
    if (path.match(/^\/erc20\/fusion-plus\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = cache.getFusionPlusTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /erc20/fusion/:chainId/:address - Get fusion labeled transfers for an address (backwards compat)
    if (path.match(/^\/erc20\/fusion\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = cache.getFusionPlusTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // =========================================================================
    // Fusion (Single-Chain) Endpoints
    // =========================================================================

    // GET /fusion/swap/:orderHash - Get swap by order hash
    if (path.match(/^\/fusion\/swap\/0x[a-fA-F0-9]{64}$/)) {
      const orderHash = path.split('/')[3];
      const swap = cache.getFusionSwap(orderHash);
      if (swap) {
        return sendResponse(res, 200, { success: true, data: swap });
      }
      return sendResponse(res, 404, { success: false, error: 'Swap not found' });
    }

    // GET /fusion/maker/:address - Get swaps by maker address
    if (path.match(/^\/fusion\/maker\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = cache.getFusionSwapsByMaker(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/taker/:address - Get swaps by taker address (recipient of output tokens)
    if (path.match(/^\/fusion\/taker\/0x[a-fA-F0-9]{40}$/)) {
      const address = path.split('/')[3];
      const swaps = cache.getFusionSwapsByTaker(address);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/chain/:chainId - Get swaps by chain
    if (path.match(/^\/fusion\/chain\/\d+$/)) {
      const chainId = parseInt(path.split('/')[3]);
      const swaps = cache.getFusionSwapsByChain(chainId);
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/filled - Get filled swaps
    if (path === '/fusion/filled') {
      const swaps = cache.getFusionSwapsByStatus('filled');
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/cancelled - Get cancelled swaps
    if (path === '/fusion/cancelled') {
      const swaps = cache.getFusionSwapsByStatus('cancelled');
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /fusion/recent - Get recent swaps
    if (path === '/fusion/recent') {
      const swaps = cache.getRecentFusionSwaps();
      return sendResponse(res, 200, { success: true, data: swaps });
    }

    // GET /erc20/fusion-single/:chainId/:address - Get fusion-labeled transfers (single-chain)
    if (path.match(/^\/erc20\/fusion-single\/\d+\/0x[a-fA-F0-9]{40}$/)) {
      const [, , , chainIdStr, address] = path.split('/');
      const chainId = parseInt(chainIdStr);
      const transfers = cache.getFusionTransfersByAddress(chainId, address);
      return sendResponse(res, 200, { success: true, data: transfers });
    }

    // GET /stats - Get database statistics
    if (path === '/stats') {
      const stats = cache.getStats();
      return sendResponse(res, 200, { success: true, data: stats });
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

function startServer(): void {
  console.log(`✅ SQLite connected: ${sqlitePath}`);

  const PORT = process.env.API_PORT || 3000;
  const server = http.createServer(handleRequest);

  server.listen(PORT, () => {
    console.log(`🚀 API Server running on http://localhost:${PORT}`);
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
    console.log('\n  Native Transfers (not supported in SQLite mode):');
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
  });

  // Graceful shutdown
  process.on('SIGINT', () => {
    console.log('\n⏸️  Shutting down API server...');
    server.close();
    cache.close();
    console.log('👋 Shutdown complete');
    process.exit(0);
  });
}

try {
  startServer();
} catch (error: any) {
  console.error('❌ Failed to start API server:', error);
  process.exit(1);
}
