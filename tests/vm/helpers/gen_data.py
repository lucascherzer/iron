#!/usr/bin/env python3
"""
Deterministic data generator for iron VM tests.

Generates pseudo-random data using a seeded RNG for reproducible testing.
Both sender and receiver can independently compute the expected hash.
"""

import argparse
import hashlib
import random
import sys
from typing import BinaryIO


def generate_data(
    seed: int,
    size: int,
    output: BinaryIO = sys.stdout.buffer,
    chunk_size: int = 4096,
) -> str:
    """
    Generate deterministic data and write to output.

    Args:
        seed: Random seed for deterministic generation
        size: Total size in bytes to generate
        output: Output stream to write data to
        chunk_size: Size of each chunk to generate/write

    Returns:
        SHA256 hash of generated data (hex string)
    """
    random.seed(seed)
    hasher = hashlib.sha256()
    remaining = size

    while remaining > 0:
        current_chunk_size = min(chunk_size, remaining)
        chunk = bytes([random.randint(0, 255) for _ in range(current_chunk_size)])
        output.write(chunk)
        output.flush()
        hasher.update(chunk)
        remaining -= current_chunk_size

    return hasher.hexdigest()


def compute_hash_only(seed: int, size: int, chunk_size: int = 4096) -> str:
    """
    Compute expected hash without generating output.

    Useful for pre-computing expected hashes on receiver side.

    Args:
        seed: Random seed for deterministic generation
        size: Total size in bytes
        chunk_size: Size of each chunk

    Returns:
        SHA256 hash (hex string)
    """
    random.seed(seed)
    hasher = hashlib.sha256()
    remaining = size

    while remaining > 0:
        current_chunk_size = min(chunk_size, remaining)
        chunk = bytes([random.randint(0, 255) for _ in range(current_chunk_size)])
        hasher.update(chunk)
        remaining -= current_chunk_size

    return hasher.hexdigest()


def parse_size(size_str: str) -> int:
    """
    Parse human-readable size string to bytes.

    Supports: 1K, 1M, 1G suffixes (base 1024)

    Args:
        size_str: Size string (e.g., "10M", "1024", "5K")

    Returns:
        Size in bytes

    Examples:
        >>> parse_size("1024")
        1024
        >>> parse_size("1K")
        1024
        >>> parse_size("10M")
        10485760
    """
    size_str = size_str.strip().upper()
    multipliers = {"K": 1024, "M": 1024**2, "G": 1024**3}

    if size_str[-1] in multipliers:
        return int(size_str[:-1]) * multipliers[size_str[-1]]
    return int(size_str)


def main():
    parser = argparse.ArgumentParser(
        description="Generate deterministic pseudo-random data for testing",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Generate 10MB with seed 42, output to stdout
  %(prog)s --seed 42 --size 10M > data.bin

  # Compute hash without generating data
  %(prog)s --seed 42 --size 10M --hash-only

  # Generate and print hash to stderr
  %(prog)s --seed 42 --size 1M 2>&1 >/dev/null
        """,
    )

    parser.add_argument(
        "--seed",
        type=int,
        required=True,
        help="Random seed for deterministic generation",
    )

    parser.add_argument(
        "--size",
        type=str,
        required=True,
        help="Size to generate (supports K, M, G suffixes)",
    )

    parser.add_argument(
        "--hash-only",
        action="store_true",
        help="Only compute and print hash, don't generate output",
    )

    parser.add_argument(
        "--chunk-size",
        type=int,
        default=4096,
        help="Chunk size for generation (default: 4096)",
    )

    args = parser.parse_args()

    try:
        size = parse_size(args.size)
    except ValueError as e:
        print(f"Error: Invalid size '{args.size}': {e}", file=sys.stderr)
        sys.exit(1)

    if args.hash_only:
        # Only compute hash
        hash_hex = compute_hash_only(args.seed, size, args.chunk_size)
        print(hash_hex)
    else:
        # Generate data and output hash to stderr
        hash_hex = generate_data(args.seed, size, sys.stdout.buffer, args.chunk_size)
        print(hash_hex, file=sys.stderr)


if __name__ == "__main__":
    main()
