#!/bin/bash

# Interactive test script for iron
# Run this in a separate terminal while iron is running

set -e

echo "=========================================="
echo "  iron Interactive Testing Suite"
echo "=========================================="
echo ""

# Check if iron is running
if ! sudo lsof -i :5333 > /dev/null 2>&1; then
    echo "❌ ERROR: iron is not running (DNS server not on port 5333)"
    echo ""
    echo "Please start iron first:"
    echo "  sudo ./target/release/iron --log-level debug"
    echo ""
    exit 1
fi

echo "✓ iron is running (DNS server detected on port 5333)"
echo ""

# Check TUN interface
echo "Checking TUN interface..."
TUN_DEV=$(ifconfig | grep "^utun" | tail -1 | awk '{print $1}' | sed 's/://')

if [ -z "$TUN_DEV" ]; then
    echo "❌ No utun device found"
    exit 1
fi

echo "✓ TUN device found: $TUN_DEV"
echo ""

# Show TUN interface details
echo "TUN Interface Details:"
echo "----------------------"
ifconfig $TUN_DEV | grep -E "flags|inet6|mtu"
echo ""

# Check IPv6 address
IPV6=$(ifconfig $TUN_DEV | grep "inet6 fd69:726f::1" | awk '{print $2}')
if [ -z "$IPV6" ]; then
    echo "❌ IPv6 address not configured on $TUN_DEV"
    exit 1
fi

echo "✓ IPv6 address configured: $IPV6"
echo ""

# Check routing
echo "Checking routing table..."
if netstat -rn -f inet6 | grep -q "fd69:726f"; then
    echo "✓ Route found:"
    netstat -rn -f inet6 | grep "fd69:726f"
else
    echo "⚠️  No route found in routing table"
fi
echo ""

# Test 1: Ping local TUN interface
echo "=========================================="
echo "Test 1: Ping Local TUN Interface"
echo "=========================================="
echo "Command: ping6 -c 3 fd69:726f::1"
echo ""

if ping6 -c 3 -W 1000 fd69:726f::1 > /dev/null 2>&1; then
    echo "✓ Successfully pinged local TUN interface!"
else
    echo "⚠️  Ping to local TUN interface failed (this might be normal)"
fi
echo ""

# Get Node ID from user
echo "=========================================="
echo "Test 2: DNS Resolution"
echo "=========================================="
echo ""
echo "To test DNS, we need your Node ID."
echo "Check the iron terminal - it should display:"
echo "  Node ID: <64-character hex string>"
echo ""
read -p "Enter your Node ID (or press Enter to skip): " NODE_ID

if [ -n "$NODE_ID" ]; then
    echo ""
    echo "Testing DNS resolution for: $NODE_ID.iron"
    echo "Command: dig @127.0.0.1 -p 5333 $NODE_ID.iron AAAA +short"
    echo ""
    
    RESOLVED_IP=$(dig @127.0.0.1 -p 5333 +short $NODE_ID.iron AAAA)
    
    if [ -n "$RESOLVED_IP" ]; then
        echo "✓ DNS Resolution successful!"
        echo "  Node ID:  $NODE_ID"
        echo "  Resolved: $RESOLVED_IP"
        echo ""
        
        # Try to ping the resolved address
        echo "Attempting to ping resolved address..."
        if ping6 -c 3 -W 1000 $RESOLVED_IP > /dev/null 2>&1; then
            echo "✓ Successfully pinged $RESOLVED_IP!"
        else
            echo "⚠️  Ping to $RESOLVED_IP failed"
            echo "   (This is expected for self-ping in P2P mode)"
        fi
    else
        echo "❌ DNS resolution failed"
        echo "   Check iron logs for errors"
    fi
else
    echo "Skipped DNS test"
fi
echo ""

# Summary
echo "=========================================="
echo "Test Summary"
echo "=========================================="
echo ""
echo "✓ iron is running"
echo "✓ TUN interface created ($TUN_DEV)"
echo "✓ IPv6 address configured (fd69:726f::1)"

if netstat -rn -f inet6 | grep -q "fd69:726f"; then
    echo "✓ Routing configured"
else
    echo "⚠️  Routing not found"
fi

echo ""
echo "Next Steps:"
echo "-----------"
echo "1. Watch iron logs for packet activity"
echo "2. Try two-node testing (see MANUAL_TESTS.md)"
echo "3. Test with real applications (HTTP server, SSH, etc.)"
echo ""
echo "To see detailed logs:"
echo "  sudo ./target/release/iron --log-level trace"
echo ""
