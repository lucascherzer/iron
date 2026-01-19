#!/bin/bash

# Helper script to convert Node ID (hex) to DNS-compatible format (base32)

if [ $# -eq 0 ]; then
    echo "Usage: $0 <NODE_ID>"
    echo ""
    echo "Converts a hex Node ID to base32 format for DNS queries."
    echo ""
    echo "Example:"
    echo "  $0 74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9"
    echo ""
    exit 1
fi

NODE_ID_HEX="$1"

# Validate hex string
if ! echo "$NODE_ID_HEX" | grep -qE '^[0-9a-fA-F]{64}$'; then
    echo "Error: Node ID must be a 64-character hexadecimal string"
    echo "Got: $NODE_ID_HEX"
    exit 1
fi

# Use Python to convert hex to base32
BASE32=$(python3 -c "
import sys
import base64

# Read hex string
hex_str = '$NODE_ID_HEX'

# Convert hex to bytes
node_id_bytes = bytes.fromhex(hex_str)

# Convert to base32 (no padding)
base32_str = base64.b32encode(node_id_bytes).decode('ascii').rstrip('=').lower()

print(base32_str)
")

if [ $? -ne 0 ]; then
    echo "Error: Failed to convert to base32"
    exit 1
fi

echo "Node ID (hex):    $NODE_ID_HEX"
echo "Node ID (base32): $BASE32"
echo ""
echo "DNS name:         $BASE32.iron"
echo ""
echo "Test DNS lookup:"
echo "  dig @127.0.0.1 -p 5333 $BASE32.iron AAAA"
echo ""
echo "Resolved IPv6 address:"
dig @127.0.0.1 -p 5333 +short "$BASE32.iron" AAAA 2>/dev/null

if [ $? -eq 0 ]; then
    echo ""
    echo "✓ DNS resolution successful!"
else
    echo ""
    echo "⚠️  DNS resolution failed (is iron running?)"
fi
