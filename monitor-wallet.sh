#!/bin/bash

WALLET="0x6E76502cf3a5CAF3e7A2E3774c8B2B5cCCe4aE99"
WALLET_LOWER=$(echo "$WALLET" | tr '[:upper:]' '[:lower:]')

echo "🔍 Monitoring wallet: $WALLET"
echo "================================"
echo ""

while true; do
  echo "$(date '+%Y-%m-%d %H:%M:%S') - Checking all networks..."

  # Check each network
  for CHAIN_ID in 1 42161 137 10 8453 100 56 43114 59144 130 1868 146 57073; do
    # Check ERC20 transfers TO this address
    RESPONSE=$(curl -s "http://localhost:3000/erc20/to/$CHAIN_ID/$WALLET_LOWER")
    COUNT=$(echo "$RESPONSE" | jq -r '.data | length' 2>/dev/null || echo "0")

    if [ "$COUNT" != "0" ] && [ "$COUNT" != "null" ]; then
      echo "  ✅ Chain $CHAIN_ID: Found $COUNT ERC20 transfers TO wallet"
      echo "$RESPONSE" | jq -r '.data[] | "    - Token: \(.token) | From: \(.from) | Value: \(.value) | Block: \(.blockNumber)"'
    fi

    # Check native transfers TO this address
    NATIVE_RESPONSE=$(curl -s "http://localhost:3000/native/to/$CHAIN_ID/$WALLET_LOWER")
    NATIVE_COUNT=$(echo "$NATIVE_RESPONSE" | jq -r '.data | length' 2>/dev/null || echo "0")

    if [ "$NATIVE_COUNT" != "0" ] && [ "$NATIVE_COUNT" != "null" ]; then
      echo "  ✅ Chain $CHAIN_ID: Found $NATIVE_COUNT native transfers TO wallet"
      echo "$NATIVE_RESPONSE" | jq -r '.data[] | "    - From: \(.from) | Value: \(.value) | Block: \(.blockNumber)"'
    fi
  done

  echo ""
  echo "Waiting 10 seconds before next check..."
  sleep 10
done
