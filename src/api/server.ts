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
    console.log('  GET /erc20/from/:chainId/:address');
    console.log('  GET /erc20/to/:chainId/:address');
    console.log('  GET /erc20/both/:chainId/:from/:to');
    console.log('  GET /erc20/address/:chainId/:address');
    console.log('  GET /native/from/:chainId/:address (returns empty - not supported)');
    console.log('  GET /native/to/:chainId/:address (returns empty - not supported)');
    console.log('  GET /native/both/:chainId/:from/:to (returns empty - not supported)');
    console.log('  GET /native/address/:chainId/:address (returns empty - not supported)');
    console.log('  GET /all/:chainId/:address');
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
