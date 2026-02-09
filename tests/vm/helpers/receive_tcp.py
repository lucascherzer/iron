#!/usr/bin/env python3
"""
TCP receiver with hash computation for iron VM tests.

Receives data over TCP, computes SHA256 hash, and outputs the hash.
"""

import argparse
import hashlib
import socket
import sys
from typing import Optional


def receive_data(
    port: int,
    expected_size: Optional[int] = None,
    bind_address: str = "::",
    timeout: Optional[int] = None,
) -> tuple[str, int]:
    """
    Receive data over TCP and compute hash.

    Args:
        port: Port to listen on
        expected_size: Expected data size (optional, for progress)
        bind_address: Address to bind to (default: :: for IPv6 any)
        timeout: Socket timeout in seconds (optional)

    Returns:
        Tuple of (hash_hex, bytes_received)
    """
    # Create IPv6 socket
    sock = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)

    if timeout:
        sock.settimeout(timeout)

    try:
        sock.bind((bind_address, port))
        sock.listen(1)

        print(f"Listening on [{bind_address}]:{port}...", file=sys.stderr, flush=True)

        conn, addr = sock.accept()
        print(f"Connection from {addr}", file=sys.stderr, flush=True)

        hasher = hashlib.sha256()
        total_received = 0

        # Receive data in chunks
        while True:
            data = conn.recv(65536)
            if not data:
                break

            hasher.update(data)
            total_received += len(data)

            # Optional progress reporting
            if expected_size and total_received % (1024 * 1024) == 0:
                progress = (total_received / expected_size) * 100
                print(
                    f"Progress: {total_received}/{expected_size} bytes ({progress:.1f}%)",
                    file=sys.stderr,
                    flush=True,
                )

        conn.close()
        print(f"Received {total_received} bytes total", file=sys.stderr, flush=True)

        return hasher.hexdigest(), total_received

    finally:
        sock.close()


def main():
    parser = argparse.ArgumentParser(
        description="Receive data over TCP and compute SHA256 hash",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Receive on port 9999, print hash to stdout
  %(prog)s --port 9999

  # Receive with expected size for progress
  %(prog)s --port 9999 --expected-size 10M

  # Bind to specific address
  %(prog)s --port 9999 --bind fd69:726f::1
        """,
    )

    parser.add_argument(
        "--port",
        type=int,
        required=True,
        help="Port to listen on",
    )

    parser.add_argument(
        "--expected-size",
        type=str,
        help="Expected data size (for progress, supports K/M/G suffixes)",
    )

    parser.add_argument(
        "--bind",
        type=str,
        default="::",
        help="Address to bind to (default: :: for IPv6 any)",
    )

    parser.add_argument(
        "--timeout",
        type=int,
        help="Socket timeout in seconds",
    )

    args = parser.parse_args()

    # Parse expected size if provided
    expected_size = None
    if args.expected_size:
        from gen_data import parse_size

        try:
            expected_size = parse_size(args.expected_size)
        except ValueError as e:
            print(f"Error: Invalid size '{args.expected_size}': {e}", file=sys.stderr)
            sys.exit(1)

    try:
        hash_hex, bytes_received = receive_data(
            args.port,
            expected_size,
            args.bind,
            args.timeout,
        )

        # Output hash to stdout
        print(hash_hex)

    except socket.timeout:
        print("Error: Connection timed out", file=sys.stderr)
        sys.exit(1)
    except OSError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
