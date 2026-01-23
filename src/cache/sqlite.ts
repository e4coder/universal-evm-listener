import Database from 'better-sqlite3';
import * as fs from 'fs';
import * as path from 'path';

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
  taker?: string;  // Address that receives taker_token (may differ from maker)
  maker_token?: string;
  taker_token?: string;
  maker_amount?: string;
  taker_amount?: string;
  remaining: string;
  is_partial_fill: number;
  status: string;
  created_at: number;
}

interface Crypto2FiatEvent {
  id: number;
  order_id: string;      // bytes32 unique order ID
  token: string;         // ERC20 token address (or 0x0 for ETH)
  amount: string;        // Amount transferred (hex)
  recipient: string;     // C2F provider address
  metadata: string;      // JSON-encoded fiat details (currencies, rates, etc.)
  chain_id: number;
  tx_hash: string;
  block_number: number;
  block_timestamp: number;
  log_index: number;
  created_at: number;
}

// =========================================================================
// Streaming/Batch interfaces for since_id pagination
// =========================================================================

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
 * SQLite cache client for the Node.js API
 * Reads from multiple SQLite databases populated by the Rust listener:
 * - Per-chain databases (chain_X.db) for transfers and checkpoints
 * - Shared database (shared.db) for Fusion+, Fusion, and Crypto2Fiat data
 */
export class SQLiteCache {
  private chainDbs: Map<number, Database.Database>;
  private sharedDb: Database.Database;

  constructor(dataDir: string) {
    this.chainDbs = new Map();

    // Open per-chain databases
    for (const chainId of CHAIN_IDS) {
      const dbPath = path.join(dataDir, `chain_${chainId}.db`);
      if (fs.existsSync(dbPath)) {
        const db = new Database(dbPath, { readonly: true });
        db.pragma('journal_mode = WAL');
        this.chainDbs.set(chainId, db);
      }
    }

    // Open shared database
    const sharedPath = path.join(dataDir, 'shared.db');
    this.sharedDb = new Database(sharedPath, { readonly: true });
    this.sharedDb.pragma('journal_mode = WAL');
  }

  // Get chain database, returns null if not available
  private getChainDb(chainId: number): Database.Database | null {
    return this.chainDbs.get(chainId) || null;
  }

  // ERC20 Queries

  getERC20TransfersByFrom(chainId: number, from: string): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE from_addr = ?
      ORDER BY block_timestamp DESC
      LIMIT 1000
    `);
    return stmt.all(chainId, from.toLowerCase());
  }

  getERC20TransfersByTo(chainId: number, to: string): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE to_addr = ?
      ORDER BY block_timestamp DESC
      LIMIT 1000
    `);
    return stmt.all(chainId, to.toLowerCase());
  }

  getERC20TransfersByBoth(chainId: number, from: string, to: string): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE from_addr = ? AND to_addr = ?
      ORDER BY block_timestamp DESC
      LIMIT 1000
    `);
    return stmt.all(chainId, from.toLowerCase(), to.toLowerCase());
  }

  // Native transfer queries (not supported in SQLite-only mode)
  // The Rust listener only captures ERC20 Transfer events

  getNativeTransfersByFrom(_chainId: number, _from: string): any[] {
    // Native transfers are not captured in SQLite-only mode
    // Return empty array for backwards compatibility
    return [];
  }

  getNativeTransfersByTo(_chainId: number, _to: string): any[] {
    return [];
  }

  getNativeTransfersByBoth(_chainId: number, _from: string, _to: string): any[] {
    return [];
  }

  // Health check

  isHealthy(): boolean {
    try {
      // Check shared database
      this.sharedDb.prepare('SELECT 1').get();

      // Check at least one chain database
      for (const db of this.chainDbs.values()) {
        db.prepare('SELECT 1').get();
        return true;
      }
      return true;
    } catch {
      return false;
    }
  }

  // Get stats

  getStats(): { transferCount: number; fusionPlusCount: number; fusionCount: number; crypto2fiatCount: number } {
    // Sum transfers from all chain databases
    let transferCount = 0;
    for (const db of this.chainDbs.values()) {
      try {
        const stmt = db.prepare('SELECT COUNT(*) as count FROM transfers');
        const result = stmt.get() as { count: number };
        transferCount += result.count;
      } catch {
        // Table might not exist yet
      }
    }

    // Get counts from shared database
    let fusionPlusCount = 0;
    try {
      const fusionPlusStmt = this.sharedDb.prepare('SELECT COUNT(*) as count FROM fusion_plus_swaps');
      const fusionPlusResult = fusionPlusStmt.get() as { count: number };
      fusionPlusCount = fusionPlusResult.count;
    } catch {
      // Table might not exist yet
    }

    let fusionCount = 0;
    try {
      const fusionStmt = this.sharedDb.prepare('SELECT COUNT(*) as count FROM fusion_swaps');
      const fusionResult = fusionStmt.get() as { count: number };
      fusionCount = fusionResult.count;
    } catch {
      // Table might not exist yet
    }

    let crypto2fiatCount = 0;
    try {
      const crypto2fiatStmt = this.sharedDb.prepare('SELECT COUNT(*) as count FROM crypto2fiat_events');
      const crypto2fiatResult = crypto2fiatStmt.get() as { count: number };
      crypto2fiatCount = crypto2fiatResult.count;
    } catch {
      // Table might not exist yet
    }

    return {
      transferCount,
      fusionPlusCount,
      fusionCount,
      crypto2fiatCount,
    };
  }

  // =========================================================================
  // Fusion+ Query Methods (from shared database)
  // =========================================================================

  // Get Fusion+ swap by order_hash
  getFusionPlusSwap(orderHash: string): FusionPlusSwap | null {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps WHERE order_hash = ?
      `);
      return (stmt.get(orderHash.toLowerCase()) as FusionPlusSwap) || null;
    } catch {
      return null;
    }
  }

  // Get Fusion+ swaps by address (as maker or taker on either chain)
  getFusionPlusSwapsByAddress(address: string, limit: number = 100): FusionPlusSwap[] {
    try {
      const addr = address.toLowerCase();
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps
        WHERE src_maker = ? OR dst_maker = ? OR src_taker = ? OR dst_taker = ?
        ORDER BY created_at DESC
        LIMIT ?
      `);
      return stmt.all(addr, addr, addr, addr, limit) as FusionPlusSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion+ swaps by status
  getFusionPlusSwapsByStatus(srcStatus: string, dstStatus: string, limit: number = 100): FusionPlusSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps
        WHERE src_status = ? AND dst_status = ?
        ORDER BY created_at DESC
        LIMIT ?
      `);
      return stmt.all(srcStatus, dstStatus, limit) as FusionPlusSwap[];
    } catch {
      return [];
    }
  }

  // Get pending Fusion+ swaps (dst escrow not yet created)
  getFusionPlusPending(limit: number = 100): FusionPlusSwap[] {
    return this.getFusionPlusSwapsByStatus('created', 'pending', limit);
  }

  // Get completed Fusion+ swaps (both sides withdrawn)
  getFusionPlusCompleted(limit: number = 100): FusionPlusSwap[] {
    return this.getFusionPlusSwapsByStatus('withdrawn', 'withdrawn', limit);
  }

  // Get Fusion+ swaps by source chain
  getFusionPlusSwapsBySrcChain(chainId: number, limit: number = 100): FusionPlusSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps
        WHERE src_chain_id = ?
        ORDER BY src_block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(chainId, limit) as FusionPlusSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion+ swaps by destination chain
  getFusionPlusSwapsByDstChain(chainId: number, limit: number = 100): FusionPlusSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps
        WHERE dst_chain_id = ?
        ORDER BY dst_block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(chainId, limit) as FusionPlusSwap[];
    } catch {
      return [];
    }
  }

  // Get swap details for a specific transfer transaction
  getSwapForTransfer(chainId: number, txHash: string): FusionPlusSwap | null {
    try {
      const hash = txHash.toLowerCase();
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_plus_swaps
        WHERE (src_chain_id = ? AND src_tx_hash = ?)
           OR (dst_chain_id = ? AND dst_tx_hash = ?)
      `);
      return (stmt.get(chainId, hash, chainId, hash) as FusionPlusSwap) || null;
    } catch {
      return null;
    }
  }

  // Get ERC20 transfers with swap_type included
  getERC20TransfersWithSwapType(chainId: number, address: string, limit: number = 1000): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, address.toLowerCase(), address.toLowerCase(), limit);
  }

  // Get transfers filtered by swap type
  getTransfersBySwapType(chainId: number, swapType: string, limit: number = 1000): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE swap_type = ?
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, swapType, limit);
  }

  // Get Fusion+ labeled transfers for an address
  getFusionPlusTransfersByAddress(chainId: number, address: string, limit: number = 1000): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const addr = address.toLowerCase();
    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE swap_type = 'fusion_plus' AND (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, addr, addr, limit);
  }

  // =========================================================================
  // Fusion (Single-Chain) Query Methods (from shared database)
  // =========================================================================

  // Get Fusion swap by order_hash
  getFusionSwap(orderHash: string): FusionSwap | null {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps WHERE order_hash = ?
        ORDER BY block_timestamp DESC LIMIT 1
      `);
      return (stmt.get(orderHash.toLowerCase()) as FusionSwap) || null;
    } catch {
      return null;
    }
  }

  // Get Fusion swaps by maker address
  getFusionSwapsByMaker(maker: string, limit: number = 100): FusionSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps
        WHERE maker = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(maker.toLowerCase(), limit) as FusionSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion swaps by taker address (recipient of output tokens)
  getFusionSwapsByTaker(taker: string, limit: number = 100): FusionSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps
        WHERE taker = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(taker.toLowerCase(), limit) as FusionSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion swaps by chain
  getFusionSwapsByChain(chainId: number, limit: number = 100): FusionSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps
        WHERE chain_id = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(chainId, limit) as FusionSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion swaps by status
  getFusionSwapsByStatus(status: string, limit: number = 100): FusionSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps
        WHERE status = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(status, limit) as FusionSwap[];
    } catch {
      return [];
    }
  }

  // Get recent Fusion swaps
  getRecentFusionSwaps(limit: number = 100): FusionSwap[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM fusion_swaps
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(limit) as FusionSwap[];
    } catch {
      return [];
    }
  }

  // Get Fusion labeled transfers for an address
  getFusionTransfersByAddress(chainId: number, address: string, limit: number = 1000): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const addr = address.toLowerCase();
    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE swap_type = 'fusion' AND (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, addr, addr, limit);
  }

  // =========================================================================
  // Crypto2Fiat Query Methods (from shared database)
  // =========================================================================

  // Get Crypto2Fiat event by order_id
  getCrypto2FiatByOrderId(orderId: string): Crypto2FiatEvent | null {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM crypto2fiat_events WHERE order_id = ?
      `);
      return (stmt.get(orderId.toLowerCase()) as Crypto2FiatEvent) || null;
    } catch {
      return null;
    }
  }

  // Get Crypto2Fiat events by recipient address
  getCrypto2FiatByRecipient(recipient: string, limit: number = 100): Crypto2FiatEvent[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM crypto2fiat_events
        WHERE recipient = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(recipient.toLowerCase(), limit) as Crypto2FiatEvent[];
    } catch {
      return [];
    }
  }

  // Get Crypto2Fiat events by chain
  getCrypto2FiatByChain(chainId: number, limit: number = 100): Crypto2FiatEvent[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM crypto2fiat_events
        WHERE chain_id = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(chainId, limit) as Crypto2FiatEvent[];
    } catch {
      return [];
    }
  }

  // Get Crypto2Fiat events by token
  getCrypto2FiatByToken(token: string, limit: number = 100): Crypto2FiatEvent[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM crypto2fiat_events
        WHERE token = ?
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(token.toLowerCase(), limit) as Crypto2FiatEvent[];
    } catch {
      return [];
    }
  }

  // Get recent Crypto2Fiat events
  getRecentCrypto2FiatEvents(limit: number = 100): Crypto2FiatEvent[] {
    try {
      const stmt = this.sharedDb.prepare(`
        SELECT * FROM crypto2fiat_events
        ORDER BY block_timestamp DESC
        LIMIT ?
      `);
      return stmt.all(limit) as Crypto2FiatEvent[];
    } catch {
      return [];
    }
  }

  // Get Crypto2Fiat labeled transfers for an address
  getCrypto2FiatTransfersByAddress(chainId: number, address: string, limit: number = 1000): any[] {
    const db = this.getChainDb(chainId);
    if (!db) return [];

    const addr = address.toLowerCase();
    const stmt = db.prepare(`
      SELECT ? as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE swap_type = 'crypto_to_fiat' AND (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, addr, addr, limit);
  }

  // =========================================================================
  // Streaming/Batch Methods (since_id pagination)
  // =========================================================================

  // Stream transfers with since_id cursor for efficient polling
  getERC20TransfersStream(
    chainId: number,
    address: string,
    options: StreamOptions = {}
  ): StreamResult {
    const db = this.getChainDb(chainId);
    if (!db) return { transfers: [], nextSinceId: options.sinceId || 0, hasMore: false };

    const sinceId = options.sinceId || 0;
    const limit = Math.min(options.limit || 100, 1000);
    const direction = options.direction || 'both';
    const addr = address.toLowerCase();

    let sql: string;
    let params: any[];

    if (direction === 'from') {
      sql = `
        SELECT id, ? as chainId, tx_hash as txHash, token, from_addr as "from",
               to_addr as "to", value, block_number as blockNumber,
               block_timestamp as timestamp, swap_type as swapType
        FROM transfers
        WHERE from_addr = ? AND id > ?
        ORDER BY id ASC
        LIMIT ?
      `;
      params = [chainId, addr, sinceId, limit + 1];
    } else if (direction === 'to') {
      sql = `
        SELECT id, ? as chainId, tx_hash as txHash, token, from_addr as "from",
               to_addr as "to", value, block_number as blockNumber,
               block_timestamp as timestamp, swap_type as swapType
        FROM transfers
        WHERE to_addr = ? AND id > ?
        ORDER BY id ASC
        LIMIT ?
      `;
      params = [chainId, addr, sinceId, limit + 1];
    } else {
      sql = `
        SELECT id, ? as chainId, tx_hash as txHash, token, from_addr as "from",
               to_addr as "to", value, block_number as blockNumber,
               block_timestamp as timestamp, swap_type as swapType
        FROM transfers
        WHERE (from_addr = ? OR to_addr = ?) AND id > ?
        ORDER BY id ASC
        LIMIT ?
      `;
      params = [chainId, addr, addr, sinceId, limit + 1];
    }

    const stmt = db.prepare(sql);
    const rows = stmt.all(...params) as TransferWithId[];

    const hasMore = rows.length > limit;
    const transfers = hasMore ? rows.slice(0, limit) : rows;
    const nextSinceId = transfers.length > 0
      ? transfers[transfers.length - 1].id
      : sinceId;

    return { transfers, nextSinceId, hasMore };
  }

  // Batch fetch transfers for multiple addresses, each with its own since_id
  getERC20TransfersBatch(
    chainId: number,
    queries: BatchQuery[],
    limit: number = 50,
    direction: 'from' | 'to' | 'both' = 'both'
  ): { [address: string]: StreamResult } {
    const results: { [address: string]: StreamResult } = {};
    const safeLimit = Math.min(limit, 100);

    for (const query of queries) {
      results[query.address.toLowerCase()] = this.getERC20TransfersStream(
        chainId,
        query.address,
        { sinceId: query.sinceId, limit: safeLimit, direction }
      );
    }

    return results;
  }

  // Close all connections

  close(): void {
    for (const db of this.chainDbs.values()) {
      db.close();
    }
    this.sharedDb.close();
  }
}
