import { SQLiteCache } from '../cache/sqlite';

export class QueryService {
  private cache: SQLiteCache;

  constructor(cache: SQLiteCache) {
    this.cache = cache;
  }

  // ERC20 Queries

  async getERC20TransfersByFrom(chainId: number, fromAddress: string): Promise<any[]> {
    return this.cache.getERC20TransfersByFrom(chainId, fromAddress);
  }

  async getERC20TransfersByTo(chainId: number, toAddress: string): Promise<any[]> {
    return this.cache.getERC20TransfersByTo(chainId, toAddress);
  }

  async getERC20TransfersByBoth(
    chainId: number,
    fromAddress: string,
    toAddress: string
  ): Promise<any[]> {
    return this.cache.getERC20TransfersByBoth(chainId, fromAddress, toAddress);
  }

  async getERC20TransfersByAddress(chainId: number, address: string): Promise<any[]> {
    const fromTransfers = await this.cache.getERC20TransfersByFrom(chainId, address);
    const toTransfers = await this.cache.getERC20TransfersByTo(chainId, address);

    // Combine and deduplicate
    const transfersMap = new Map<string, any>();

    [...fromTransfers, ...toTransfers].forEach((transfer) => {
      const key = `${transfer.txHash}:${transfer.token}:${transfer.from}:${transfer.to}`;
      transfersMap.set(key, transfer);
    });

    return Array.from(transfersMap.values()).sort((a, b) => b.timestamp - a.timestamp);
  }

  // Native Transfer Queries

  async getNativeTransfersByFrom(chainId: number, fromAddress: string): Promise<any[]> {
    return this.cache.getNativeTransfersByFrom(chainId, fromAddress);
  }

  async getNativeTransfersByTo(chainId: number, toAddress: string): Promise<any[]> {
    return this.cache.getNativeTransfersByTo(chainId, toAddress);
  }

  async getNativeTransfersByBoth(
    chainId: number,
    fromAddress: string,
    toAddress: string
  ): Promise<any[]> {
    return this.cache.getNativeTransfersByBoth(chainId, fromAddress, toAddress);
  }

  async getNativeTransfersByAddress(chainId: number, address: string): Promise<any[]> {
    const fromTransfers = await this.cache.getNativeTransfersByFrom(chainId, address);
    const toTransfers = await this.cache.getNativeTransfersByTo(chainId, address);

    // Combine and deduplicate
    const transfersMap = new Map<string, any>();

    [...fromTransfers, ...toTransfers].forEach((transfer) => {
      const key = `${transfer.txHash}:${transfer.from}:${transfer.to}`;
      transfersMap.set(key, transfer);
    });

    return Array.from(transfersMap.values()).sort((a, b) => b.timestamp - a.timestamp);
  }

  // Combined Queries

  async getAllTransfersByAddress(chainId: number, address: string): Promise<any> {
    const erc20Transfers = await this.getERC20TransfersByAddress(chainId, address);
    const nativeTransfers = await this.getNativeTransfersByAddress(chainId, address);

    return {
      erc20: erc20Transfers,
      native: nativeTransfers,
      total: erc20Transfers.length + nativeTransfers.length,
    };
  }
}
