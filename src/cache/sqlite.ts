import Database from 'better-sqlite3';

interface Transfer {
  chain_id: number;
  tx_hash: string;
  log_index: number;
  token: string;
  from_addr: string;
  to_addr: string;
  value: string;
  block_number: number;
  block_timestamp: number;
  swap_type?: string;
}

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

/**
 * SQLite cache client for the Node.js API
 * Reads from the SQLite database populated by the Rust listener
 */
export class SQLiteCache {
  private db: Database.Database;

  constructor(dbPath: string) {
    this.db = new Database(dbPath, { readonly: true });
    // Enable WAL mode for better concurrent read performance
    this.db.pragma('journal_mode = WAL');
  }

  // ERC20 Queries

  getERC20TransfersByFrom(chainId: number, from: string): any[] {
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp
      FROM transfers
      WHERE chain_id = ? AND from_addr = ?
      ORDER BY block_timestamp DESC
      LIMIT 1000
    `);
    return stmt.all(chainId, from.toLowerCase());
  }

  getERC20TransfersByTo(chainId: number, to: string): any[] {
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp
      FROM transfers
      WHERE chain_id = ? AND to_addr = ?
      ORDER BY block_timestamp DESC
      LIMIT 1000
    `);
    return stmt.all(chainId, to.toLowerCase());
  }

  getERC20TransfersByBoth(chainId: number, from: string, to: string): any[] {
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from", to_addr as "to",
             value, block_number as blockNumber, block_timestamp as timestamp
      FROM transfers
      WHERE chain_id = ? AND from_addr = ? AND to_addr = ?
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
      this.db.prepare('SELECT 1').get();
      return true;
    } catch {
      return false;
    }
  }

  // Get stats

  getStats(): { transferCount: number; fusionPlusCount: number } {
    const transferStmt = this.db.prepare('SELECT COUNT(*) as count FROM transfers');
    const transferResult = transferStmt.get() as { count: number };

    let fusionPlusCount = 0;
    try {
      const fusionStmt = this.db.prepare('SELECT COUNT(*) as count FROM fusion_plus_swaps');
      const fusionResult = fusionStmt.get() as { count: number };
      fusionPlusCount = fusionResult.count;
    } catch {
      // Table might not exist yet
    }

    return {
      transferCount: transferResult.count,
      fusionPlusCount,
    };
  }

  // =========================================================================
  // Fusion+ Query Methods
  // =========================================================================

  // Get Fusion+ swap by order_hash
  getFusionPlusSwap(orderHash: string): FusionPlusSwap | null {
    try {
      const stmt = this.db.prepare(`
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
      const stmt = this.db.prepare(`
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
      const stmt = this.db.prepare(`
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
      const stmt = this.db.prepare(`
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
      const stmt = this.db.prepare(`
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
      const stmt = this.db.prepare(`
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
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE chain_id = ? AND (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, address.toLowerCase(), address.toLowerCase(), limit);
  }

  // Get transfers filtered by swap type
  getTransfersBySwapType(chainId: number, swapType: string, limit: number = 1000): any[] {
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE chain_id = ? AND swap_type = ?
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, swapType, limit);
  }

  // Get Fusion+ labeled transfers for an address
  getFusionPlusTransfersByAddress(chainId: number, address: string, limit: number = 1000): any[] {
    const addr = address.toLowerCase();
    const stmt = this.db.prepare(`
      SELECT chain_id as chainId, tx_hash as txHash, token, from_addr as "from",
             to_addr as "to", value, block_number as blockNumber,
             block_timestamp as timestamp, swap_type as swapType
      FROM transfers
      WHERE chain_id = ? AND swap_type = 'fusion_plus' AND (from_addr = ? OR to_addr = ?)
      ORDER BY block_timestamp DESC
      LIMIT ?
    `);
    return stmt.all(chainId, addr, addr, limit);
  }

  // Close connection

  close(): void {
    this.db.close();
  }
}
