#!/bin/bash

# Minimal TUN test - must run with sudo

if [ "$EUID" -ne 0 ]; then 
    echo "ERROR: Must run with sudo"
    exit 1
fi

echo "Building minimal TUN test..."
cargo build --release --example test_tun 2>&1 | tail -5

echo ""
echo "Running test (this will fail without sudo)..."
./target/release/examples/test_tun

echo ""
echo "Checking for TUN devices..."
ifconfig | grep utun | tail -3
