import { Pool, PoolClient } from 'pg';

// All supported chain IDs
const CHAIN_IDS = [1, 10, 56, 100, 130, 137, 146, 1868, 8453, 42161, 43114, 57073, 59144];

interface FusionPlusSwap {
  id: number;
  order_hash: string;
  hashlock: string;
  secret?: string;
  src_chain_id: number;
  src_tx_hash: string;
  src_block_number: number;
  src_block_timestamp: number;
  src_log_index: number;
  src_escrow_address?: string;
  src_maker: string;
  src_taker: string;
  src_token: string;
  src_amount: string;
  src_safety_deposit: string;
  src_timelocks: string;
  src_status: string;
  dst_chain_id: number;
  dst_tx_hash?: string;
  dst_block_number?: number;
  dst_block_timestamp?: number;
  dst_log_index?: number;
  dst_escrow_address?: string;
  dst_maker: string;
  dst_taker?: string;
  dst_token: string;
  dst_amount: string;
  dst_safety_deposit: string;
  dst_timelocks?: string;
  dst_status: string;
  created_at: number;
  updated_at: number;
}

interface FusionSwap {
  id: number;
  order_hash: string;
  chain_id: number;
  tx_hash: string;
  block_number: number;
  block_timestamp: number;
  log_index: number;
  maker: string;
  taker?: string;
  maker_token?: string;
  taker_token?: string;
  maker_amount?: string;
  taker_amount?: string;
  remaining: string;
  is_partial_fill: boolean;
  status: string;
  created_at: number;
}

interface Crypto2FiatEvent {
  id: number;
  order_id: string;
  token: string;
  amount: string;
  recipient: string;
  metadata: string;
  chain_id: number;
  tx_hash: string;
  block_number: number;
  block_timestamp: number;
  log_index: number;
  created_at: number;
}

// Streaming/Batch interfaces for since_id pagination
interface StreamOptions {
  sinceId?: number;
  limit?: number;
  direction?: 'from' | 'to' | 'both';
}

interface TransferWithId {
  id: number;
  chainId: number;
  txHash: string;
  token: string;
  from: string;
  to: string;
  value: string;
  blockNumber: number;
  timestamp: number;
  swapType: string | null;
}

interface StreamResult {
  transfers: TransferWithId[];
  nextSinceId: number;
  hasMore: boolean;
}

interface BatchQuery {
  address: string;
  sinceId: number;
}

/**
 * PostgreSQL cache client for the Node.js API
 * Reads from a single PostgreSQL database populated by the Rust listener
 */
export class PostgresCache {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({
      connectionString: databaseUrl,
      max: 20,
      idleTimeoutMillis: 30000,
      connectionTimeoutMillis: 2000,
    });
  }

  // ERC20 Queries

  async getERC20TransfersByFrom(chainId: number, from: string): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from", to_addr as "to",
              value, block_number as "blockNumber", block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND from_addr = $2
       ORDER BY block_timestamp DESC
       LIMIT 1000`,
      [chainId, from.toLowerCase()]
    );
    return result.rows;
  }

  async getERC20TransfersByTo(chainId: number, to: string): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from", to_addr as "to",
              value, block_number as "blockNumber", block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND to_addr = $2
       ORDER BY block_timestamp DESC
       LIMIT 1000`,
      [chainId, to.toLowerCase()]
    );
    return result.rows;
  }

  async getERC20TransfersByBoth(chainId: number, from: string, to: string): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from", to_addr as "to",
              value, block_number as "blockNumber", block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND from_addr = $2 AND to_addr = $3
       ORDER BY block_timestamp DESC
       LIMIT 1000`,
      [chainId, from.toLowerCase(), to.toLowerCase()]
    );
    return result.rows;
  }

  // Native transfer queries (not supported - Rust listener only captures ERC20 Transfer events)

  async getNativeTransfersByFrom(_chainId: number, _from: string): Promise<any[]> {
    return [];
  }

  async getNativeTransfersByTo(_chainId: number, _to: string): Promise<any[]> {
    return [];
  }

  async getNativeTransfersByBoth(_chainId: number, _from: string, _to: string): Promise<any[]> {
    return [];
  }

  // Health check

  async isHealthy(): Promise<boolean> {
    try {
      await this.pool.query('SELECT 1');
      return true;
    } catch {
      return false;
    }
  }

  // Get stats

  async getStats(): Promise<{ transferCount: number; fusionPlusCount: number; fusionCount: number; crypto2fiatCount: number }> {
    try {
      const [transfers, fusionPlus, fusion, crypto2fiat] = await Promise.all([
        this.pool.query('SELECT COUNT(*)::int as count FROM transfers'),
        this.pool.query('SELECT COUNT(*)::int as count FROM fusion_plus_swaps'),
        this.pool.query('SELECT COUNT(*)::int as count FROM fusion_swaps'),
        this.pool.query('SELECT COUNT(*)::int as count FROM crypto2fiat_events'),
      ]);

      return {
        transferCount: transfers.rows[0]?.count || 0,
        fusionPlusCount: fusionPlus.rows[0]?.count || 0,
        fusionCount: fusion.rows[0]?.count || 0,
        crypto2fiatCount: crypto2fiat.rows[0]?.count || 0,
      };
    } catch {
      return { transferCount: 0, fusionPlusCount: 0, fusionCount: 0, crypto2fiatCount: 0 };
    }
  }

  // =========================================================================
  // Fusion+ Query Methods
  // =========================================================================

  async getFusionPlusSwap(orderHash: string): Promise<FusionPlusSwap | null> {
    try {
      const result = await this.pool.query(
        'SELECT * FROM fusion_plus_swaps WHERE order_hash = $1',
        [orderHash.toLowerCase()]
      );
      return result.rows[0] || null;
    } catch {
      return null;
    }
  }

  async getFusionPlusSwapsByAddress(address: string, limit: number = 100): Promise<FusionPlusSwap[]> {
    try {
      const addr = address.toLowerCase();
      const result = await this.pool.query(
        `SELECT * FROM fusion_plus_swaps
         WHERE src_maker = $1 OR dst_maker = $1 OR src_taker = $1 OR dst_taker = $1
         ORDER BY created_at DESC
         LIMIT $2`,
        [addr, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionPlusSwapsByStatus(srcStatus: string, dstStatus: string, limit: number = 100): Promise<FusionPlusSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_plus_swaps
         WHERE src_status = $1 AND dst_status = $2
         ORDER BY created_at DESC
         LIMIT $3`,
        [srcStatus, dstStatus, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionPlusPending(limit: number = 100): Promise<FusionPlusSwap[]> {
    return this.getFusionPlusSwapsByStatus('created', 'pending', limit);
  }

  async getFusionPlusCompleted(limit: number = 100): Promise<FusionPlusSwap[]> {
    return this.getFusionPlusSwapsByStatus('withdrawn', 'withdrawn', limit);
  }

  async getFusionPlusSwapsBySrcChain(chainId: number, limit: number = 100): Promise<FusionPlusSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_plus_swaps
         WHERE src_chain_id = $1
         ORDER BY src_block_timestamp DESC
         LIMIT $2`,
        [chainId, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionPlusSwapsByDstChain(chainId: number, limit: number = 100): Promise<FusionPlusSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_plus_swaps
         WHERE dst_chain_id = $1
         ORDER BY dst_block_timestamp DESC
         LIMIT $2`,
        [chainId, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getSwapForTransfer(chainId: number, txHash: string): Promise<FusionPlusSwap | null> {
    try {
      const hash = txHash.toLowerCase();
      const result = await this.pool.query(
        `SELECT * FROM fusion_plus_swaps
         WHERE (src_chain_id = $1 AND src_tx_hash = $2)
            OR (dst_chain_id = $1 AND dst_tx_hash = $2)`,
        [chainId, hash]
      );
      return result.rows[0] || null;
    } catch {
      return null;
    }
  }

  async getERC20TransfersWithSwapType(chainId: number, address: string, limit: number = 1000): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
              to_addr as "to", value, block_number as "blockNumber",
              block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND (from_addr = $2 OR to_addr = $2)
       ORDER BY block_timestamp DESC
       LIMIT $3`,
      [chainId, address.toLowerCase(), limit]
    );
    return result.rows;
  }

  async getTransfersBySwapType(chainId: number, swapType: string, limit: number = 1000): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
              to_addr as "to", value, block_number as "blockNumber",
              block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND swap_type = $2
       ORDER BY block_timestamp DESC
       LIMIT $3`,
      [chainId, swapType, limit]
    );
    return result.rows;
  }

  async getFusionPlusTransfersByAddress(chainId: number, address: string, limit: number = 1000): Promise<any[]> {
    const addr = address.toLowerCase();
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
              to_addr as "to", value, block_number as "blockNumber",
              block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND swap_type = 'fusion_plus' AND (from_addr = $2 OR to_addr = $2)
       ORDER BY block_timestamp DESC
       LIMIT $3`,
      [chainId, addr, limit]
    );
    return result.rows;
  }

  // =========================================================================
  // Fusion (Single-Chain) Query Methods
  // =========================================================================

  async getFusionSwap(orderHash: string): Promise<FusionSwap | null> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps WHERE order_hash = $1
         ORDER BY block_timestamp DESC LIMIT 1`,
        [orderHash.toLowerCase()]
      );
      return result.rows[0] || null;
    } catch {
      return null;
    }
  }

  async getFusionSwapsByMaker(maker: string, limit: number = 100): Promise<FusionSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps
         WHERE maker = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [maker.toLowerCase(), limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionSwapsByTaker(taker: string, limit: number = 100): Promise<FusionSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps
         WHERE taker = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [taker.toLowerCase(), limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionSwapsByChain(chainId: number, limit: number = 100): Promise<FusionSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps
         WHERE chain_id = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [chainId, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionSwapsByStatus(status: string, limit: number = 100): Promise<FusionSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps
         WHERE status = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [status, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getRecentFusionSwaps(limit: number = 100): Promise<FusionSwap[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM fusion_swaps
         ORDER BY block_timestamp DESC
         LIMIT $1`,
        [limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getFusionTransfersByAddress(chainId: number, address: string, limit: number = 1000): Promise<any[]> {
    const addr = address.toLowerCase();
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
              to_addr as "to", value, block_number as "blockNumber",
              block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND swap_type = 'fusion' AND (from_addr = $2 OR to_addr = $2)
       ORDER BY block_timestamp DESC
       LIMIT $3`,
      [chainId, addr, limit]
    );
    return result.rows;
  }

  // =========================================================================
  // Crypto2Fiat Query Methods
  // =========================================================================

  async getCrypto2FiatByOrderId(orderId: string): Promise<Crypto2FiatEvent | null> {
    try {
      const result = await this.pool.query(
        'SELECT * FROM crypto2fiat_events WHERE order_id = $1',
        [orderId.toLowerCase()]
      );
      return result.rows[0] || null;
    } catch {
      return null;
    }
  }

  async getCrypto2FiatByRecipient(recipient: string, limit: number = 100): Promise<Crypto2FiatEvent[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM crypto2fiat_events
         WHERE recipient = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [recipient.toLowerCase(), limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getCrypto2FiatByChain(chainId: number, limit: number = 100): Promise<Crypto2FiatEvent[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM crypto2fiat_events
         WHERE chain_id = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [chainId, limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getCrypto2FiatByToken(token: string, limit: number = 100): Promise<Crypto2FiatEvent[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM crypto2fiat_events
         WHERE token = $1
         ORDER BY block_timestamp DESC
         LIMIT $2`,
        [token.toLowerCase(), limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getRecentCrypto2FiatEvents(limit: number = 100): Promise<Crypto2FiatEvent[]> {
    try {
      const result = await this.pool.query(
        `SELECT * FROM crypto2fiat_events
         ORDER BY block_timestamp DESC
         LIMIT $1`,
        [limit]
      );
      return result.rows;
    } catch {
      return [];
    }
  }

  async getCrypto2FiatTransfersByAddress(chainId: number, address: string, limit: number = 1000): Promise<any[]> {
    const addr = address.toLowerCase();
    const result = await this.pool.query(
      `SELECT $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
              to_addr as "to", value, block_number as "blockNumber",
              block_timestamp as "timestamp", swap_type as "swapType"
       FROM transfers
       WHERE chain_id = $1 AND swap_type = 'crypto_to_fiat' AND (from_addr = $2 OR to_addr = $2)
       ORDER BY block_timestamp DESC
       LIMIT $3`,
      [chainId, addr, limit]
    );
    return result.rows;
  }

  // =========================================================================
  // Streaming/Batch Methods (since_id pagination)
  // =========================================================================

  async getERC20TransfersStream(
    chainId: number,
    address: string,
    options: StreamOptions = {}
  ): Promise<StreamResult> {
    const sinceId = options.sinceId || 0;
    const limit = Math.min(options.limit || 100, 1000);
    const direction = options.direction || 'both';
    const addr = address.toLowerCase();

    let sql: string;
    let params: any[];

    if (direction === 'from') {
      sql = `
        SELECT id, $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
               to_addr as "to", value, block_number as "blockNumber",
               block_timestamp as "timestamp", swap_type as "swapType"
        FROM transfers
        WHERE chain_id = $1 AND from_addr = $2 AND id > $3
        ORDER BY id ASC
        LIMIT $4
      `;
      params = [chainId, addr, sinceId, limit + 1];
    } else if (direction === 'to') {
      sql = `
        SELECT id, $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
               to_addr as "to", value, block_number as "blockNumber",
               block_timestamp as "timestamp", swap_type as "swapType"
        FROM transfers
        WHERE chain_id = $1 AND to_addr = $2 AND id > $3
        ORDER BY id ASC
        LIMIT $4
      `;
      params = [chainId, addr, sinceId, limit + 1];
    } else {
      sql = `
        SELECT id, $1::int as "chainId", tx_hash as "txHash", token, from_addr as "from",
               to_addr as "to", value, block_number as "blockNumber",
               block_timestamp as "timestamp", swap_type as "swapType"
        FROM transfers
        WHERE chain_id = $1 AND (from_addr = $2 OR to_addr = $2) AND id > $3
        ORDER BY id ASC
        LIMIT $4
      `;
      params = [chainId, addr, sinceId, limit + 1];
    }

    const result = await this.pool.query(sql, params);
    const rows = result.rows as TransferWithId[];

    const hasMore = rows.length > limit;
    const transfers = hasMore ? rows.slice(0, limit) : rows;
    const nextSinceId = transfers.length > 0
      ? transfers[transfers.length - 1].id
      : sinceId;

    return { transfers, nextSinceId, hasMore };
  }

  async getERC20TransfersBatch(
    chainId: number,
    queries: BatchQuery[],
    limit: number = 50,
    direction: 'from' | 'to' | 'both' = 'both'
  ): Promise<{ [address: string]: StreamResult }> {
    const results: { [address: string]: StreamResult } = {};
    const safeLimit = Math.min(limit, 100);

    // Execute queries in parallel
    const promises = queries.map(async (query) => {
      const result = await this.getERC20TransfersStream(
        chainId,
        query.address,
        { sinceId: query.sinceId, limit: safeLimit, direction }
      );
      return { address: query.address.toLowerCase(), result };
    });

    const resolved = await Promise.all(promises);
    for (const { address, result } of resolved) {
      results[address] = result;
    }

    return results;
  }

  // Close connection pool

  async close(): Promise<void> {
    await this.pool.end();
  }
}
