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

  getStats(): { transferCount: number } {
    const stmt = this.db.prepare('SELECT COUNT(*) as count FROM transfers');
    const result = stmt.get() as { count: number };
    return { transferCount: result.count };
  }

  // Close connection

  close(): void {
    this.db.close();
  }
}
