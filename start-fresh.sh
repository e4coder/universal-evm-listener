#!/bin/bash

echo "🛑 Stopping all existing processes..."
pkill -f "node dist/index.js" 2>/dev/null || true
pkill -f "node test-single-network.js" 2>/dev/null || true
sleep 2

echo "🗑️  Clearing Redis..."
docker exec universal-listener-redis redis-cli FLUSHALL

echo "🚀 Starting Universal Blockchain Listener (13 networks)..."
npm start

