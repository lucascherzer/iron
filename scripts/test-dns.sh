#!/bin/bash

# Simple iron test - extracts Node ID from logs and tests DNS

echo "=========================================="
echo "  iron DNS Test"
echo "=========================================="
echo ""

# Check if iron is running
if ! sudo lsof -i :5333 > /dev/null 2>&1; then
    echo "❌ iron is not running"
    echo ""
    echo "Please start iron first:"
    echo "  sudo ./target/release/iron --log-level debug"
    exit 1
fi

echo "✓ iron is running"
echo ""

# Check TUN interface
TUN_DEV=$(ifconfig | grep "^utun" | tail -1 | awk '{print $1}' | sed 's/://')
if [ -n "$TUN_DEV" ]; then
    echo "✓ TUN device: $TUN_DEV"
    
    # Check IPv6
    IPV6=$(ifconfig $TUN_DEV | grep "inet6 fd69:726f::1" | awk '{print $2}')
    if [ -n "$IPV6" ]; then
        echo "✓ IPv6 address: $IPV6"
    fi
else
    echo "⚠️  No TUN device found"
fi

echo ""
echo "=========================================="
echo "DNS Test"
echo "=========================================="
echo ""
echo "The iron binary now displays the Node ID in both formats:"
echo "  - Hex (64 chars) - for reference"
echo "  - Base32 (52 chars) - for DNS queries"
echo ""
echo "Look for these lines in iron's output:"
echo "  Node ID (base32): <52-char-string>"
echo "  DNS name:         <52-char-string>.iron"
echo ""
read -p "Enter the base32 Node ID from iron's logs: " BASE32_ID

if [ -z "$BASE32_ID" ]; then
    echo "No Node ID provided, skipping DNS test"
    exit 0
fi

echo ""
echo "Testing DNS resolution for: $BASE32_ID.iron"
echo ""

RESULT=$(dig @127.0.0.1 -p 5333 +short "$BASE32_ID.iron" AAAA 2>&1)

if [ -n "$RESULT" ]; then
    echo "✓ DNS Resolution successful!"
    echo "  Query:    $BASE32_ID.iron"
    echo "  Resolved: $RESULT"
    echo ""
    
    # Try to ping
    echo "Attempting to ping the resolved address..."
    if ping6 -c 3 -W 1000 "$RESULT" > /dev/null 2>&1; then
        echo "✓ Ping successful!"
    else
        echo "⚠️  Ping failed (this is normal for self-ping in P2P mode)"
    fi
else
    echo "❌ DNS resolution failed"
    echo ""
    echo "Troubleshooting:"
    echo "  1. Make sure you copied the base32 Node ID (not hex)"
    echo "  2. Check iron's logs for errors"
    echo "  3. Try: dig @127.0.0.1 -p 5333 $BASE32_ID.iron AAAA"
fi

echo ""
echo "=========================================="
echo "Done!"
echo "=========================================="
