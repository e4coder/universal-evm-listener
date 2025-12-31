import { Network } from 'alchemy-sdk';

export interface NetworkConfig {
  name: string;
  chainId: number;
  alchemyNetwork: Network;
  nativeSymbol: string;
}

export const SUPPORTED_NETWORKS: NetworkConfig[] = [
  // {
  //   name: 'Ethereum',
  //   chainId: 1,
  //   alchemyNetwork: Network.ETH_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  {
    name: 'Arbitrum One',
    chainId: 42161,
    alchemyNetwork: Network.ARB_MAINNET,
    nativeSymbol: 'ETH',
  },
  // {
  //   name: 'Polygon',
  //   chainId: 137,
  //   alchemyNetwork: Network.MATIC_MAINNET,
  //   nativeSymbol: 'MATIC',
  // },
  // {
  //   name: 'OP Mainnet',
  //   chainId: 10,
  //   alchemyNetwork: Network.OPT_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  // {
  //   name: 'Base',
  //   chainId: 8453,
  //   alchemyNetwork: Network.BASE_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  // {
  //   name: 'Gnosis',
  //   chainId: 100,
  //   alchemyNetwork: Network.GNOSIS_MAINNET,
  //   nativeSymbol: 'xDAI',
  // },
  // {
  //   name: 'BNB Smart Chain',
  //   chainId: 56,
  //   alchemyNetwork: Network.BNB_MAINNET,
  //   nativeSymbol: 'BNB',
  // },
  // {
  //   name: 'Avalanche',
  //   chainId: 43114,
  //   alchemyNetwork: Network.AVAX_MAINNET,
  //   nativeSymbol: 'AVAX',
  // },
  // {
  //   name: 'Linea Mainnet',
  //   chainId: 59144,
  //   alchemyNetwork: Network.LINEA_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  // {
  //   name: 'Unichain',
  //   chainId: 130,
  //   alchemyNetwork: Network.UNICHAIN_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  // {
  //   name: 'Soneium Mainnet',
  //   chainId: 1868,
  //   alchemyNetwork: Network.SONEIUM_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
  // {
  //   name: 'Sonic',
  //   chainId: 146,
  //   alchemyNetwork: Network.SONIC_MAINNET,
  //   nativeSymbol: 'S',
  // },
  // {
  //   name: 'Ink',
  //   chainId: 57073,
  //   alchemyNetwork: Network.INK_MAINNET,
  //   nativeSymbol: 'ETH',
  // },
];

export function getNetworkConfig(chainId: number): NetworkConfig | undefined {
  return SUPPORTED_NETWORKS.find((network) => network.chainId === chainId);
}
