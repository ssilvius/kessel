#!/usr/bin/env python3
"""GOM binary format analyzer for SWTOR game objects."""

import base64
import struct
import sqlite3
import sys
from pathlib import Path


def analyze_payload(payload: bytes, fqn: str) -> dict:
    """Analyze a GOM payload and extract structure info."""
    result = {
        'fqn': fqn,
        'size': len(payload),
        'type_markers': {},
        'strings': [],
        'references': [],
    }

    # Count type marker frequencies (0xC0-0xDF range)
    for b in payload:
        if 0xC0 <= b <= 0xDF:
            result['type_markers'][b] = result['type_markers'].get(b, 0) + 1

    # Find FQN location
    fqn_bytes = fqn.encode('utf-8')
    try:
        fqn_start = payload.index(fqn_bytes[:10])
        fqn_end = payload.index(b'\x00', fqn_start)
        result['fqn_offset'] = fqn_start
        result['data_offset'] = fqn_end + 1
    except ValueError:
        result['fqn_offset'] = -1
        result['data_offset'] = 0

    # Extract embedded strings (length-prefixed)
    i = result.get('data_offset', 0)
    while i < len(payload) - 4:
        length = payload[i]
        if 4 < length < 150 and i + 1 + length <= len(payload):
            potential = payload[i+1:i+1+length]
            if all(32 <= b < 127 for b in potential):
                try:
                    s = potential.decode('ascii')
                    if '.' in s or '_' in s:  # FQN-like patterns
                        result['strings'].append((i, s))
                        i += 1 + length
                        continue
                except:
                    pass
        i += 1

    # Look for 0xCF (u64) patterns - likely object references
    i = result.get('data_offset', 0)
    while i < len(payload) - 9:
        if payload[i] == 0xCF:
            val = struct.unpack('<Q', payload[i+1:i+9])[0]
            if val > 0x1000000:  # Likely a reference, not a small int
                result['references'].append((i, val))
        i += 1

    return result


def print_analysis(result: dict):
    """Print analysis results."""
    print(f"\n{'='*70}")
    print(f"FQN: {result['fqn']}")
    print(f"Size: {result['size']} bytes")
    print(f"FQN offset: {result.get('fqn_offset', -1)}")
    print(f"Data offset: {result.get('data_offset', 0)}")

    print(f"\nType markers (0xC0-0xDF):")
    markers = sorted(result['type_markers'].items(), key=lambda x: -x[1])[:10]
    for marker, count in markers:
        names = {
            0xCA: 'int32?', 0xCB: 'ref32?', 0xCC: 'field?',
            0xCD: 'int16?', 0xCE: 'int32?', 0xCF: 'u64/ref',
        }
        print(f"  0x{marker:02X} ({names.get(marker, 'unknown'):8s}): {count:4d}")

    print(f"\nEmbedded strings ({len(result['strings'])} found):")
    for offset, s in result['strings'][:20]:
        print(f"  {offset:5d}: {s[:70]}")

    print(f"\nU64 references ({len(result['references'])} found):")
    for offset, val in result['references'][:15]:
        print(f"  {offset:5d}: 0x{val:016X}")


def main():
    db_path = sys.argv[1] if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'

    conn = sqlite3.connect(db_path)

    # Analyze a few different object types
    queries = [
        "SELECT json_extract(json, '$.payload_b64'), fqn FROM objects WHERE fqn LIKE 'qst.location.korriban.class.sith_warrior.the_final%' LIMIT 1",
        "SELECT json_extract(json, '$.payload_b64'), fqn FROM objects WHERE fqn LIKE 'abl.%' AND kind = 'Ability' LIMIT 1",
        "SELECT json_extract(json, '$.payload_b64'), fqn FROM objects WHERE fqn LIKE 'itm.%' AND kind = 'Item' LIMIT 1",
        "SELECT json_extract(json, '$.payload_b64'), fqn FROM objects WHERE fqn LIKE 'npc.%' AND kind = 'Npc' LIMIT 1",
    ]

    for query in queries:
        row = conn.execute(query).fetchone()
        if row:
            b64, fqn = row
            try:
                payload = base64.b64decode(b64)
                result = analyze_payload(payload, fqn)
                print_analysis(result)
            except Exception as e:
                print(f"Error analyzing {fqn}: {e}")

    conn.close()


if __name__ == '__main__':
    main()
