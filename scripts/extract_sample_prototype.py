#!/usr/bin/env python3
"""Extract a sample prototype node to analyze its format."""

import struct
import zlib
import subprocess
from pathlib import Path
import re

# Read hash dict to find prototype node hashes
hash_path = Path.home() / 'swtor/data/hashes_filename.txt'

prototype_info = []
with open(hash_path) as f:
    for line in f:
        if '/prototypes/' in line and '.node' in line:
            parts = line.strip().split('#')
            if len(parts) >= 2:
                hash_hex = parts[0]
                hash_val = int(hash_hex, 16)
                path = parts[2] if len(parts) > 2 else ""
                prototype_info.append((hash_val, hash_hex, path))

print(f"Found {len(prototype_info)} prototype nodes")
print(f"Sample: {prototype_info[0][2]}")
print(f"  Hash: 0x{prototype_info[0][1]}")

# Find which .tor file contains this hash
assets_dir = Path.home() / 'swtor/assets'
tor_files = sorted(assets_dir.glob('*.tor'))

def read_tor_entry(tor_path, target_hash):
    """Read a single entry from a .tor file by hash."""
    with open(tor_path, 'rb') as f:
        # Read header
        magic = f.read(4)
        if magic != b'MYP\x00':
            return None

        version = struct.unpack('<I', f.read(4))[0]
        file_table_offset = struct.unpack('<Q', f.read(8))[0]
        file_table_size = struct.unpack('<Q', f.read(8))[0]
        file_count = struct.unpack('<Q', f.read(8))[0]

        # Read file table
        f.seek(file_table_offset)
        for _ in range(int(file_count)):
            data_offset = struct.unpack('<Q', f.read(8))[0]
            compressed_size = struct.unpack('<I', f.read(4))[0]
            uncompressed_size = struct.unpack('<I', f.read(4))[0]
            filename_hash = struct.unpack('<Q', f.read(8))[0]
            crc32 = struct.unpack('<I', f.read(4))[0]
            compression_type = struct.unpack('<I', f.read(4))[0]
            flags = struct.unpack('<H', f.read(2))[0]

            if filename_hash == target_hash:
                f.seek(data_offset)
                data = f.read(compressed_size)

                if compression_type == 1:  # zlib
                    data = zlib.decompress(data)
                elif compression_type == 2:  # zstd
                    # Skip zstd for now, return raw data
                    pass

                return data

    return None

# Try to find and read a prototype
target_hash = prototype_info[0][0]
print(f"\nSearching for hash 0x{target_hash:08X} in .tor files...")

for tor_path in tor_files:
    data = read_tor_entry(tor_path, target_hash)
    if data:
        print(f"\nFound in: {tor_path.name}")
        print(f"Size: {len(data)} bytes")
        print(f"First 64 bytes (hex):")
        for i in range(0, min(64, len(data)), 16):
            hex_part = ' '.join(f'{b:02X}' for b in data[i:i+16])
            ascii_part = ''.join(chr(b) if 32 <= b < 127 else '.' for b in data[i:i+16])
            print(f"  {i:04X}: {hex_part:<48} {ascii_part}")

        # Check for known magic bytes
        if data[:4] == b'PBUK':
            print("\nFormat: PBUK container")
        elif data[:4] == b'DBLB':
            print("\nFormat: DBLB (direct)")
        elif data[:4] == bytes([0x28, 0xB5, 0x2F, 0xFD]):
            print("\nFormat: ZSTD compressed")
        else:
            print(f"\nFormat: Unknown (magic: {data[:4].hex()})")

        break
else:
    print("Not found in any .tor file!")
