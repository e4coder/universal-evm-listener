#!/bin/bash

# Setup Log Rotation for Existing PM2 Deployment
# Run this on your production server

echo "================================================"
echo "  Setting Up Log Rotation for Existing Deploy"
echo "================================================"
echo ""

# Check if PM2 is running
if ! pm2 list > /dev/null 2>&1; then
    echo "❌ PM2 is not running or not installed"
    exit 1
fi

echo "✅ PM2 is running"
echo ""

# Check if pm2-logrotate is already installed
if pm2 list | grep -q "pm2-logrotate"; then
    echo "ℹ️  pm2-logrotate is already installed"
    echo "Current configuration:"
    pm2 conf pm2-logrotate
    echo ""
    read -p "Do you want to reconfigure? (y/n) " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Skipping reconfiguration"
        exit 0
    fi
fi

# Install pm2-logrotate
echo "📦 Installing pm2-logrotate..."
pm2 install pm2-logrotate
sleep 3
echo "✅ pm2-logrotate installed"
echo ""

# Configure settings
echo "⚙️  Configuring log rotation..."
pm2 set pm2-logrotate:max_size 10M
pm2 set pm2-logrotate:retain 7
pm2 set pm2-logrotate:compress true
pm2 set pm2-logrotate:dateFormat YYYY-MM-DD_HH-mm-ss
echo "✅ Configuration applied"
echo ""

# Save configuration
echo "💾 Saving PM2 configuration..."
pm2 save
echo "✅ Configuration saved"
echo ""

# Display configuration
echo "📋 Current Log Rotation Settings:"
pm2 conf pm2-logrotate
echo ""

echo "================================================"
echo "  ✅ Log Rotation Setup Complete!"
echo "================================================"
echo ""
echo "Log files will now:"
echo "  • Rotate when they reach 10MB"
echo "  • Keep 7 old rotated files (compressed)"
echo "  • Use ~160MB disk space maximum"
echo ""
echo "View logs: pm2 logs"
echo "Check rotation status: pm2 logs pm2-logrotate"
echo ""
