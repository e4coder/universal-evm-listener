import { createClient, RedisClientType } from 'redis';

export class RedisCache {
  private client: RedisClientType;
  private readonly TTL: number;

  constructor() {
    // Default to 1 hour (3600 seconds), configurable via CACHE_TTL_HOURS
    const ttlHours = process.env.CACHE_TTL_HOURS
      ? parseInt(process.env.CACHE_TTL_HOURS, 10)
      : 1;
    this.TTL = ttlHours * 60 * 60; // Convert hours to seconds

    this.client = createClient({
      url: process.env.REDIS_URL || 'redis://localhost:6379',
    });

    this.client.on('error', (err) => console.error('Redis Client Error', err));
    this.client.on('connect', () => console.log('Redis Client Connected'));
  }

  async connect(): Promise<void> {
    await this.client.connect();
  }

  async disconnect(): Promise<void> {
    await this.client.disconnect();
  }

  // Store ERC20 transfer event
  async storeERC20Transfer(
    chainId: number,
    txHash: string,
    token: string,
    from: string,
    to: string,
    value: string,
    blockNumber: number,
    timestamp: number
  ): Promise<void> {
    const transfer = {
      txHash,
      token,
      from: from.toLowerCase(),
      to: to.toLowerCase(),
      value,
      blockNumber,
      timestamp,
      chainId,
    };

    const transferKey = `transfer:erc20:${chainId}:${txHash}:${token}:${from}:${to}`;

    // Store the transfer data
    await this.client.setEx(transferKey, this.TTL, JSON.stringify(transfer));

    // Index by 'from' address
    await this.client.zAdd(
      `idx:erc20:from:${chainId}:${from.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:erc20:from:${chainId}:${from.toLowerCase()}`, this.TTL);

    // Index by 'to' address
    await this.client.zAdd(
      `idx:erc20:to:${chainId}:${to.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:erc20:to:${chainId}:${to.toLowerCase()}`, this.TTL);

    // Index by both 'from' and 'to'
    await this.client.zAdd(
      `idx:erc20:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:erc20:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`, this.TTL);
  }

  // Store native transfer (ETH, MATIC, BNB, etc.)
  async storeNativeTransfer(
    chainId: number,
    txHash: string,
    from: string,
    to: string,
    value: string,
    blockNumber: number,
    timestamp: number
  ): Promise<void> {
    const transfer = {
      txHash,
      from: from.toLowerCase(),
      to: to.toLowerCase(),
      value,
      blockNumber,
      timestamp,
      chainId,
    };

    const transferKey = `transfer:native:${chainId}:${txHash}:${from}:${to}`;

    // Store the transfer data
    await this.client.setEx(transferKey, this.TTL, JSON.stringify(transfer));

    // Index by 'from' address
    await this.client.zAdd(
      `idx:native:from:${chainId}:${from.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:native:from:${chainId}:${from.toLowerCase()}`, this.TTL);

    // Index by 'to' address
    await this.client.zAdd(
      `idx:native:to:${chainId}:${to.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:native:to:${chainId}:${to.toLowerCase()}`, this.TTL);

    // Index by both 'from' and 'to'
    await this.client.zAdd(
      `idx:native:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`,
      { score: timestamp, value: transferKey }
    );
    await this.client.expire(`idx:native:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`, this.TTL);
  }

  // Get ERC20 transfers by 'from' address
  async getERC20TransfersByFrom(chainId: number, from: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:erc20:from:${chainId}:${from.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Get ERC20 transfers by 'to' address
  async getERC20TransfersByTo(chainId: number, to: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:erc20:to:${chainId}:${to.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Get ERC20 transfers by both 'from' and 'to' addresses
  async getERC20TransfersByBoth(chainId: number, from: string, to: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:erc20:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Get native transfers by 'from' address
  async getNativeTransfersByFrom(chainId: number, from: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:native:from:${chainId}:${from.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Get native transfers by 'to' address
  async getNativeTransfersByTo(chainId: number, to: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:native:to:${chainId}:${to.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Get native transfers by both 'from' and 'to' addresses
  async getNativeTransfersByBoth(chainId: number, from: string, to: string): Promise<any[]> {
    const keys = await this.client.zRange(
      `idx:native:both:${chainId}:${from.toLowerCase()}:${to.toLowerCase()}`,
      0,
      -1
    );
    return this.getTransfersByKeys(keys);
  }

  // Helper to retrieve transfers by their keys
  private async getTransfersByKeys(keys: string[]): Promise<any[]> {
    if (keys.length === 0) return [];

    const transfers: any[] = [];
    for (const key of keys) {
      const data = await this.client.get(key);
      if (data) {
        transfers.push(JSON.parse(data));
      }
    }
    return transfers;
  }
}
