#!/bin/bash

# Test script for iron
# This script must be run with sudo

set -e

echo "========================================"
echo "  Testing iron P2P Network Interface"
echo "========================================"
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then 
    echo "ERROR: This script must be run with sudo"
    echo "Usage: sudo ./test-iron.sh"
    exit 1
fi

echo "✓ Running as root"
echo ""

# Build the binary
echo "Building iron..."
cargo build --release --quiet
echo "✓ Build successful"
echo ""

# Run iron in the background
echo "Starting iron..."
./target/release/iron --log-level info &
IRON_PID=$!

# Wait a moment for startup
sleep 2

# Check if iron is still running
if ! kill -0 $IRON_PID 2>/dev/null; then
    echo "✗ iron failed to start"
    exit 1
fi

echo "✓ iron started successfully (PID: $IRON_PID)"
echo ""

# Check if TUN device was created
if ifconfig | grep -q utun; then
    TUN_DEVICE=$(ifconfig | grep "^utun" | tail -1 | awk '{print $1}' | sed 's/://')
    echo "✓ TUN device created: $TUN_DEVICE"
    
    # Show TUN device details
    echo ""
    echo "TUN Device Details:"
    ifconfig "$TUN_DEVICE" | grep inet6 | head -5
else
    echo "✗ No TUN device found"
    kill $IRON_PID 2>/dev/null || true
    exit 1
fi

echo ""
echo "✓ iron is running successfully!"
echo ""
echo "Press Ctrl-C to stop iron..."
echo ""

# Wait for user to stop
wait $IRON_PID

echo ""
echo "✓ iron stopped"
