#\!/bin/bash
WALLET="0x6E76502cf3a5CAF3e7A2E3774c8B2B5cCCe4aE99"
echo "🔍 Monitoring wallet: $WALLET"
echo "================================"
echo ""

while true; do
  TIMESTAMP=$(date "+%H:%M:%S")
  echo "[$TIMESTAMP] Checking..."
  
  # Check Arbitrum
  RESULT=$(curl -s "http://localhost:3000/all/42161/0x6e76502cf3a5caf3e7a2e3774c8b2b5ccce4ae99")
  ERC20_COUNT=$(echo "$RESULT" | jq -r ".data.erc20 | length" 2>/dev/null || echo "0")
  NATIVE_COUNT=$(echo "$RESULT" | jq -r ".data.native | length" 2>/dev/null || echo "0")
  
  if [ "$ERC20_COUNT" \!= "0" ] || [ "$NATIVE_COUNT" \!= "0" ]; then
    echo "  ✅ Arbitrum: $ERC20_COUNT ERC20, $NATIVE_COUNT native transfers"
    if [ "$ERC20_COUNT" \!= "0" ]; then
      echo "$RESULT" | jq -r ".data.erc20[] | \"    📝 \(.txHash) - Token: \(.token) - Value: \(.value)\""
    fi
  fi
  
  sleep 10
done
