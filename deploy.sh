#!/bin/bash

# Universal Blockchain Listener - PM2 Deployment Script
# This script handles complete deployment with PM2

set -e  # Exit on error

echo "================================================"
echo "  Universal Blockchain Listener - PM2 Deploy"
echo "================================================"
echo ""

# Check if PM2 is installed
if ! command -v pm2 &> /dev/null; then
    echo "❌ PM2 is not installed. Installing..."
    npm install -g pm2
    echo "✅ PM2 installed"
fi

# Check if .env exists
if [ ! -f .env ]; then
    echo "❌ .env file not found!"
    echo "Please create .env file with your configuration:"
    echo "  cp .env.example .env"
    echo "  nano .env  # Add your ALCHEMY_API_KEY"
    exit 1
fi

# Check if ALCHEMY_API_KEY is set
if ! grep -q "ALCHEMY_API_KEY=." .env; then
    echo "❌ ALCHEMY_API_KEY not set in .env file!"
    exit 1
fi

echo "✅ Prerequisites checked"
echo ""

# Install dependencies
echo "📦 Installing dependencies..."
npm install
echo "✅ Dependencies installed"
echo ""

# Build project
echo "🔨 Building project..."
npm run build
echo "✅ Project built"
echo ""

# Create logs directory
mkdir -p logs
echo "✅ Logs directory ready"
echo ""

# Start Redis if not running
echo "🔄 Checking Redis..."
if ! docker ps | grep -q universal-listener-redis; then
    echo "Starting Redis with Docker Compose..."
    docker compose up -d
    sleep 3
    echo "✅ Redis started"
else
    echo "✅ Redis already running"
fi
echo ""

# Stop existing PM2 processes if any
echo "🛑 Stopping existing PM2 processes..."
pm2 delete ecosystem.config.js 2>/dev/null || echo "No existing processes to stop"
echo ""

# Start with PM2
echo "🚀 Starting applications with PM2..."
pm2 start ecosystem.config.js
echo "✅ Applications started"
echo ""

# Save PM2 process list
echo "💾 Saving PM2 configuration..."
pm2 save
echo "✅ PM2 configuration saved"
echo ""

# Show status
echo "📊 Current Status:"
pm2 list
echo ""

# Health checks
echo "🏥 Running health checks..."
sleep 5

# Check API
if curl -s http://localhost:5459/networks > /dev/null; then
    echo "✅ API Server: Healthy (port 5459)"
else
    echo "⚠️  API Server: Not responding yet (may need a moment to start)"
fi

# Check Redis
if docker exec universal-listener-redis redis-cli ping > /dev/null 2>&1; then
    echo "✅ Redis: Healthy"
else
    echo "❌ Redis: Not responding"
fi

echo ""
echo "================================================"
echo "  Deployment Complete! 🎉"
echo "================================================"
echo ""
echo "📋 Quick Commands:"
echo "  View logs:      npm run pm2:logs"
echo "  Monitor:        npm run pm2:monit"
echo "  Status:         npm run pm2:status"
echo "  Restart:        npm run pm2:restart"
echo "  Stop:           npm run pm2:stop"
echo ""
echo "📖 For more info, see PM2_DEPLOYMENT.md"
echo ""
