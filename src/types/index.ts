export interface ERC20Transfer {
  txHash: string;
  token: string;
  from: string;
  to: string;
  value: string;
  blockNumber: number;
  timestamp: number;
  chainId: number;
}

export interface NativeTransfer {
  txHash: string;
  from: string;
  to: string;
  value: string;
  blockNumber: number;
  timestamp: number;
  chainId: number;
}

export interface AllTransfers {
  erc20: ERC20Transfer[];
  native: NativeTransfer[];
  total: number;
}
