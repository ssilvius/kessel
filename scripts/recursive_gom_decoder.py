#!/usr/bin/env python3
"""Recursive GOM decoder - fully parse nested msgpack-like structures."""

import base64
import struct
import sqlite3
import sys
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any


@dataclass
class GomNode:
    """A decoded GOM node (can be nested)."""
    offset: int
    marker: int
    value: Any
    size: int
    children: list['GomNode'] = field(default_factory=list)

    @property
    def type_name(self) -> str:
        m = self.marker
        if m <= 0x7F:
            return 'fixint+'
        if m >= 0xE0:
            return 'fixint-'
        if 0x80 <= m <= 0x8F:
            return f'fixmap({m & 0x0F})'
        if 0x90 <= m <= 0x9F:
            return f'fixarray({m & 0x0F})'
        if 0xA0 <= m <= 0xBF:
            return f'fixstr({m & 0x1F})'
        names = {
            0xC0: 'nil',
            0xC1: 'unused',
            0xC2: 'false',
            0xC3: 'true',
            0xC4: 'bin8',
            0xC5: 'bin16',
            0xC6: 'bin32',
            0xC7: 'ext8',
            0xC8: 'ext16',
            0xC9: 'ext32',
            0xCA: 'u32',
            0xCB: 'ref32',
            0xCC: 'field',
            0xCD: 'u16',
            0xCE: 'u32_ce',
            0xCF: 'u64',
            0xD0: 'int8',
            0xD1: 'int16',
            0xD2: 'int32',
            0xD3: 'int64',
            0xD4: 'fixext1',
            0xD5: 'fixext2',
            0xD6: 'fixext4',
            0xD7: 'fixext8',
            0xD8: 'fixext16',
            0xD9: 'str8',
            0xDA: 'str16',
            0xDB: 'str32',
            0xDC: 'array16',
            0xDD: 'array32',
            0xDE: 'map16',
            0xDF: 'map32',
        }
        return names.get(m, f'unk_{m:02X}')


def decode_recursive(data: bytes, pos: int, depth: int = 0) -> tuple[GomNode | None, int]:
    """Recursively decode a GOM value with all nested children."""
    if pos >= len(data):
        return None, pos

    marker = data[pos]
    start_pos = pos

    # Positive fixint (0x00-0x7F)
    if marker <= 0x7F:
        return GomNode(start_pos, marker, marker, 1), pos + 1

    # Negative fixint (0xE0-0xFF)
    if marker >= 0xE0:
        return GomNode(start_pos, marker, marker - 256, 1), pos + 1

    # Fixmap (0x80-0x8F): N key-value pairs
    if 0x80 <= marker <= 0x8F:
        n_pairs = marker & 0x0F
        pos += 1
        children = []
        for _ in range(n_pairs * 2):  # key + value for each pair
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_pairs, pos - start_pos, children), pos

    # Fixarray (0x90-0x9F): N elements
    if 0x90 <= marker <= 0x9F:
        n_elems = marker & 0x0F
        pos += 1
        children = []
        for _ in range(n_elems):
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_elems, pos - start_pos, children), pos

    # Fixstr (0xA0-0xBF): N bytes of string
    if 0xA0 <= marker <= 0xBF:
        length = marker & 0x1F
        if pos + 1 + length > len(data):
            return None, pos + 1
        s = data[pos + 1:pos + 1 + length]
        try:
            s = s.decode('utf-8')
        except:
            s = s.hex()
        return GomNode(start_pos, marker, s, 1 + length), pos + 1 + length

    # Nil
    if marker == 0xC0:
        return GomNode(start_pos, marker, None, 1), pos + 1

    # False/True
    if marker == 0xC2:
        return GomNode(start_pos, marker, False, 1), pos + 1
    if marker == 0xC3:
        return GomNode(start_pos, marker, True, 1), pos + 1

    # bin8 (C4): 1-byte length + data
    if marker == 0xC4:
        if pos + 2 > len(data):
            return None, pos + 1
        length = data[pos + 1]
        if pos + 2 + length > len(data):
            return None, pos + 1
        return GomNode(start_pos, marker, data[pos + 2:pos + 2 + length], 2 + length), pos + 2 + length

    # bin16 (C5): 2-byte length + data
    if marker == 0xC5:
        if pos + 3 > len(data):
            return None, pos + 1
        length = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        if pos + 3 + length > len(data):
            return None, pos + 1
        return GomNode(start_pos, marker, data[pos + 3:pos + 3 + length], 3 + length), pos + 3 + length

    # bin32 (C6): 4-byte length + data
    if marker == 0xC6:
        if pos + 5 > len(data):
            return None, pos + 1
        length = struct.unpack('>I', data[pos + 1:pos + 5])[0]
        if pos + 5 + length > len(data):
            return None, pos + 1
        return GomNode(start_pos, marker, data[pos + 5:pos + 5 + length], 5 + length), pos + 5 + length

    # ext8 (C7): 1-byte length + type + data
    if marker == 0xC7:
        if pos + 3 > len(data):
            return None, pos + 1
        length = data[pos + 1]
        ext_type = data[pos + 2]
        if pos + 3 + length > len(data):
            return None, pos + 1
        ext_data = data[pos + 3:pos + 3 + length]
        return GomNode(start_pos, marker, (ext_type, ext_data), 3 + length), pos + 3 + length

    # ext16 (C8): 2-byte length + type + data
    if marker == 0xC8:
        if pos + 4 > len(data):
            return None, pos + 1
        length = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        ext_type = data[pos + 3]
        if pos + 4 + length > len(data):
            return None, pos + 1
        ext_data = data[pos + 4:pos + 4 + length]
        return GomNode(start_pos, marker, (ext_type, ext_data), 4 + length), pos + 4 + length

    # ext32 (C9): 4-byte length + type + data
    if marker == 0xC9:
        if pos + 6 > len(data):
            return None, pos + 1
        length = struct.unpack('>I', data[pos + 1:pos + 5])[0]
        ext_type = data[pos + 5]
        if pos + 6 + length > len(data):
            return None, pos + 1
        ext_data = data[pos + 6:pos + 6 + length]
        return GomNode(start_pos, marker, (ext_type, ext_data), 6 + length), pos + 6 + length

    # Custom SWTOR u32 types (CA, CB, CC, CE) - all little-endian
    if marker in (0xCA, 0xCB, 0xCC, 0xCE):
        if pos + 5 > len(data):
            return None, pos + 1
        val = struct.unpack('<I', data[pos + 1:pos + 5])[0]
        return GomNode(start_pos, marker, val, 5), pos + 5

    # Standard msgpack u16 (CD) - BIG-endian!
    if marker == 0xCD:
        if pos + 3 > len(data):
            return None, pos + 1
        val = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        return GomNode(start_pos, marker, val, 3), pos + 3

    # u64 (CF) - little-endian
    if marker == 0xCF:
        if pos + 9 > len(data):
            return None, pos + 1
        val = struct.unpack('<Q', data[pos + 1:pos + 9])[0]
        return GomNode(start_pos, marker, val, 9), pos + 9

    # int8 (D0)
    if marker == 0xD0:
        if pos + 2 > len(data):
            return None, pos + 1
        val = struct.unpack('b', data[pos + 1:pos + 2])[0]
        return GomNode(start_pos, marker, val, 2), pos + 2

    # int16 (D1)
    if marker == 0xD1:
        if pos + 3 > len(data):
            return None, pos + 1
        val = struct.unpack('>h', data[pos + 1:pos + 3])[0]
        return GomNode(start_pos, marker, val, 3), pos + 3

    # int32 (D2)
    if marker == 0xD2:
        if pos + 5 > len(data):
            return None, pos + 1
        val = struct.unpack('>i', data[pos + 1:pos + 5])[0]
        return GomNode(start_pos, marker, val, 5), pos + 5

    # int64 (D3)
    if marker == 0xD3:
        if pos + 9 > len(data):
            return None, pos + 1
        val = struct.unpack('>q', data[pos + 1:pos + 9])[0]
        return GomNode(start_pos, marker, val, 9), pos + 9

    # fixext1 (D4): type + 1 byte
    if marker == 0xD4:
        if pos + 3 > len(data):
            return None, pos + 1
        ext_type = data[pos + 1]
        ext_data = data[pos + 2:pos + 3]
        return GomNode(start_pos, marker, (ext_type, ext_data), 3), pos + 3

    # fixext2 (D5): type + 2 bytes
    if marker == 0xD5:
        if pos + 4 > len(data):
            return None, pos + 1
        ext_type = data[pos + 1]
        ext_data = data[pos + 2:pos + 4]
        return GomNode(start_pos, marker, (ext_type, ext_data), 4), pos + 4

    # fixext4 (D6): type + 4 bytes
    if marker == 0xD6:
        if pos + 6 > len(data):
            return None, pos + 1
        ext_type = data[pos + 1]
        ext_data = data[pos + 2:pos + 6]
        return GomNode(start_pos, marker, (ext_type, ext_data), 6), pos + 6

    # fixext8 (D7): type + 8 bytes
    if marker == 0xD7:
        if pos + 10 > len(data):
            return None, pos + 1
        ext_type = data[pos + 1]
        ext_data = data[pos + 2:pos + 10]
        return GomNode(start_pos, marker, (ext_type, ext_data), 10), pos + 10

    # fixext16 (D8): type + 16 bytes
    if marker == 0xD8:
        if pos + 18 > len(data):
            return None, pos + 1
        ext_type = data[pos + 1]
        ext_data = data[pos + 2:pos + 18]
        return GomNode(start_pos, marker, (ext_type, ext_data), 18), pos + 18

    # str8 (D9): 1-byte length + string
    if marker == 0xD9:
        if pos + 2 > len(data):
            return None, pos + 1
        length = data[pos + 1]
        if pos + 2 + length > len(data):
            return None, pos + 1
        s = data[pos + 2:pos + 2 + length]
        try:
            s = s.decode('utf-8')
        except:
            s = s.hex()
        return GomNode(start_pos, marker, s, 2 + length), pos + 2 + length

    # str16 (DA): 2-byte length + string
    if marker == 0xDA:
        if pos + 3 > len(data):
            return None, pos + 1
        length = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        if pos + 3 + length > len(data):
            return None, pos + 1
        s = data[pos + 3:pos + 3 + length]
        try:
            s = s.decode('utf-8')
        except:
            s = s.hex()
        return GomNode(start_pos, marker, s, 3 + length), pos + 3 + length

    # str32 (DB): 4-byte length + string
    if marker == 0xDB:
        if pos + 5 > len(data):
            return None, pos + 1
        length = struct.unpack('>I', data[pos + 1:pos + 5])[0]
        if pos + 5 + length > len(data):
            return None, pos + 1
        s = data[pos + 5:pos + 5 + length]
        try:
            s = s.decode('utf-8')
        except:
            s = s.hex()
        return GomNode(start_pos, marker, s, 5 + length), pos + 5 + length

    # array16 (DC): 2-byte count + elements
    if marker == 0xDC:
        if pos + 3 > len(data):
            return None, pos + 1
        n_elems = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        pos += 3
        children = []
        for _ in range(n_elems):
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_elems, pos - start_pos, children), pos

    # array32 (DD): 4-byte count + elements
    if marker == 0xDD:
        if pos + 5 > len(data):
            return None, pos + 1
        n_elems = struct.unpack('>I', data[pos + 1:pos + 5])[0]
        pos += 5
        children = []
        for _ in range(min(n_elems, 10000)):  # Safety limit
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_elems, pos - start_pos, children), pos

    # map16 (DE): 2-byte count + key-value pairs
    if marker == 0xDE:
        if pos + 3 > len(data):
            return None, pos + 1
        n_pairs = struct.unpack('>H', data[pos + 1:pos + 3])[0]
        pos += 3
        children = []
        for _ in range(n_pairs * 2):
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_pairs, pos - start_pos, children), pos

    # map32 (DF): 4-byte count + key-value pairs
    if marker == 0xDF:
        if pos + 5 > len(data):
            return None, pos + 1
        n_pairs = struct.unpack('>I', data[pos + 1:pos + 5])[0]
        pos += 5
        children = []
        for _ in range(min(n_pairs * 2, 20000)):  # Safety limit
            child, pos = decode_recursive(data, pos, depth + 1)
            if child is None:
                break
            children.append(child)
        return GomNode(start_pos, marker, n_pairs, pos - start_pos, children), pos

    # Fallback: unknown marker
    return GomNode(start_pos, marker, data[pos], 1), pos + 1


def find_fqn_bounds(payload: bytes) -> tuple[int, int] | None:
    """Find the start and end offsets of the FQN."""
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.', b'cnv.']:
        pos = payload.find(prefix)
        if pos > 0 and pos < 30:
            fqn_len = payload[pos - 1]
            fqn_start = pos
            fqn_end = pos - 1 + fqn_len
            if fqn_end < len(payload):
                return (fqn_start, fqn_end)
    return None


def print_tree(node: GomNode, indent: int = 0, max_depth: int = 5):
    """Print a GOM node tree."""
    prefix = "  " * indent
    val_str = str(node.value)
    if isinstance(node.value, bytes):
        val_str = node.value[:20].hex() + ('...' if len(node.value) > 20 else '')
    elif isinstance(node.value, tuple) and len(node.value) == 2:
        ext_type, ext_data = node.value
        if isinstance(ext_data, bytes):
            val_str = f"ext_type={ext_type}, data={ext_data[:20].hex()}"
    elif isinstance(node.value, str) and len(node.value) > 50:
        val_str = node.value[:50] + '...'

    print(f"{prefix}[{node.offset:5d}] {node.type_name:15s} = {val_str}")

    if node.children and indent < max_depth:
        for child in node.children[:20]:  # Limit children shown
            print_tree(child, indent + 1, max_depth)
        if len(node.children) > 20:
            print(f"{prefix}  ... ({len(node.children) - 20} more children)")


def collect_all_nodes(node: GomNode) -> list[GomNode]:
    """Flatten all nodes in the tree."""
    result = [node]
    for child in node.children:
        result.extend(collect_all_nodes(child))
    return result


def analyze_payload(payload: bytes, fqn: str, conn: sqlite3.Connection):
    """Analyze a GOM payload with recursive parsing."""
    print(f"\n{'='*80}")
    print(f"FQN: {fqn}")
    print(f"Size: {len(payload)} bytes")

    bounds = find_fqn_bounds(payload)
    if not bounds:
        print("Could not find FQN bounds")
        return

    fqn_start, fqn_end = bounds
    print(f"FQN at: {fqn_start}-{fqn_end}")

    # Parse from after FQN end
    nodes = []
    pos = fqn_end
    while pos < len(payload):
        node, new_pos = decode_recursive(payload, pos)
        if node is None or new_pos == pos:
            pos += 1
            continue
        nodes.append(node)
        pos = new_pos

    print(f"Parsed {len(nodes)} top-level nodes")

    # Collect all nodes including nested
    all_nodes = []
    for n in nodes:
        all_nodes.extend(collect_all_nodes(n))
    print(f"Total nodes (including nested): {len(all_nodes)}")

    # Type statistics
    type_counts = defaultdict(int)
    for n in all_nodes:
        type_counts[n.type_name] += 1

    print("\n--- Type Statistics ---")
    for t, c in sorted(type_counts.items(), key=lambda x: -x[1])[:30]:
        print(f"  {t:20s}: {c:5d}")

    # Show some interesting structures
    print("\n--- Sample Structures (first 20 top-level) ---")
    for node in nodes[:20]:
        print_tree(node, max_depth=3)

    # Collect all strings found
    strings = []
    for n in all_nodes:
        if n.type_name.startswith('fixstr') or n.type_name.startswith('str'):
            if isinstance(n.value, str):
                strings.append((n.offset, n.value))

    print(f"\n--- All Embedded Strings ({len(strings)} found) ---")
    for offset, s in strings[:50]:
        # Check if it exists as an FQN
        exists = conn.execute('SELECT 1 FROM objects WHERE fqn = ? LIMIT 1', (s,)).fetchone()
        marker = '[FQN EXISTS]' if exists else ''
        print(f"  {offset:5d}: {s[:60]} {marker}")

    # Analyze extension types
    ext_nodes = [n for n in all_nodes if 'ext' in n.type_name]
    if ext_nodes:
        print(f"\n--- Extension Types ({len(ext_nodes)} found) ---")
        ext_types = defaultdict(int)
        for n in ext_nodes:
            if isinstance(n.value, tuple):
                ext_type = n.value[0]
                ext_types[ext_type] += 1

        for ext_type, count in sorted(ext_types.items(), key=lambda x: -x[1]):
            print(f"  ext_type={ext_type:3d}: {count:4d} occurrences")

        # Show sample ext data
        print("\n  Sample ext data:")
        for n in ext_nodes[:10]:
            if isinstance(n.value, tuple):
                ext_type, ext_data = n.value
                data_preview = ext_data[:30].hex() if isinstance(ext_data, bytes) else str(ext_data)
                print(f"    [{n.offset:5d}] type={ext_type:3d}, data={data_preview}")


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Get a quest
    row = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn
        FROM objects
        WHERE fqn = 'qst.location.korriban.class.sith_warrior.the_final_trial'
    """).fetchone()

    if row:
        payload_b64, fqn = row
        payload = base64.b64decode(payload_b64)
        analyze_payload(payload, fqn, conn)

    conn.close()


if __name__ == '__main__':
    main()
