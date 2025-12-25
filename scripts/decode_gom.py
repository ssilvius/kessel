#!/usr/bin/env python3
"""GOM binary format decoder for SWTOR game objects.

Decodes the msgpack-like binary format used in SWTOR GOM (Game Object Model).

## GOM Payload Structure

1. Header (variable, ~14 bytes):
   - Bytes 0-1: padding (00 00) or (00 00 00 00)
   - Type ID (4 bytes)
   - Offset/count (4 bytes)
   - Other metadata

2. FQN (length-prefixed):
   - 1 byte: length
   - N bytes: FQN string (e.g., "qst.location.korriban.class.sith_warrior.the_final_trial")

3. String Table ID (+12 bytes from FQN end):
   - Pattern: CA <4b> <1b> CA <4b> [CC/CD <id2>]
   - CC <u32 LE>: for id2 < 256
   - CD <u16 BE>: for id2 >= 256 (standard msgpack big-endian!)

4. Structured Data:
   - Type markers with values
   - Embedded FQN references (npc.*, spn.*, mpn.*, cdx.*)
   - String table references: <id1> 01 06 07 str.qst CC <field>

## Type Markers (msgpack-like, but mixed endianness!)

- 0x00-0x7F: positive fixint (0-127)
- 0xE0-0xFF: negative fixint (-32 to -1)
- 0xC0: nil
- 0xC2: false
- 0xC3: true
- 0xCA: u32 value (4 bytes, LITTLE-endian) - custom
- 0xCB: u32 reference (4 bytes, LITTLE-endian) - object/field hash ref
- 0xCC: field ID (4 bytes, LITTLE-endian) - field type/name hash
- 0xCD: u16 value (2 bytes, BIG-endian) - standard msgpack!
- 0xCE: u32 value (4 bytes, LITTLE-endian) - custom
- 0xCF: u64 value (8 bytes, LITTLE-endian) - often GUID references

## String Table Lookup

Strings are stored in a separate table with (id1, id2, text) keys.
- id2 = quest/object string table ID (extracted from header)
- id1 = string index within that table
- id1=88 is typically the object name (quest title, ability name, etc.)
"""

import base64
import struct
import sqlite3
import sys
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass
from typing import Any, Iterator


@dataclass
class GomValue:
    """A decoded GOM value."""
    offset: int
    marker: int
    value: Any
    raw_bytes: bytes

    @property
    def type_name(self) -> str:
        names = {
            0xC0: 'nil',
            0xC2: 'false',
            0xC3: 'true',
            0xCA: 'u32',
            0xCB: 'ref32',
            0xCC: 'field',
            0xCD: 'u16',
            0xCE: 'u32',
            0xCF: 'u64',
        }
        if self.marker <= 0x7F:
            return 'int+'
        if self.marker >= 0xE0:
            return 'int-'
        return names.get(self.marker, f'unk_{self.marker:02X}')


def decode_value(data: bytes, pos: int) -> tuple[GomValue | None, int]:
    """Decode a single GOM value at position. Returns (value, next_pos)."""
    if pos >= len(data):
        return None, pos

    marker = data[pos]

    if marker <= 0x7F:  # positive fixint
        return GomValue(pos, marker, marker, data[pos:pos+1]), pos + 1

    if marker >= 0xE0:  # negative fixint
        return GomValue(pos, marker, marker - 256, data[pos:pos+1]), pos + 1

    if marker == 0xC0:  # nil
        return GomValue(pos, marker, None, data[pos:pos+1]), pos + 1

    if marker == 0xC2:  # false
        return GomValue(pos, marker, False, data[pos:pos+1]), pos + 1

    if marker == 0xC3:  # true
        return GomValue(pos, marker, True, data[pos:pos+1]), pos + 1

    if marker in (0xCA, 0xCB, 0xCC, 0xCE):  # 4-byte values
        if pos + 5 > len(data):
            return None, pos + 1
        val = struct.unpack('<I', data[pos+1:pos+5])[0]
        return GomValue(pos, marker, val, data[pos:pos+5]), pos + 5

    if marker == 0xCD:  # u16 - uses BIG-ENDIAN (standard msgpack)
        if pos + 3 > len(data):
            return None, pos + 1
        val = struct.unpack('>H', data[pos+1:pos+3])[0]  # Big-endian!
        return GomValue(pos, marker, val, data[pos:pos+3]), pos + 3

    if marker == 0xCF:  # u64
        if pos + 9 > len(data):
            return None, pos + 1
        val = struct.unpack('<Q', data[pos+1:pos+9])[0]
        return GomValue(pos, marker, val, data[pos:pos+9]), pos + 9

    # Unknown marker, skip
    return GomValue(pos, marker, data[pos], data[pos:pos+1]), pos + 1


def decode_all(data: bytes, start: int = 0) -> Iterator[GomValue]:
    """Decode all GOM values from data starting at offset."""
    pos = start
    while pos < len(data):
        val, next_pos = decode_value(data, pos)
        if val is None:
            break
        yield val
        pos = next_pos


def extract_strings(data: bytes) -> list[tuple[int, str]]:
    """Extract embedded string references from payload."""
    strings = []

    # Pattern: <length> str.<prefix>
    prefixes = [b'str.', b'qst.', b'npc.', b'spn.', b'mpn.', b'abl.', b'itm.', b'cdx.', b'cnv.', b'ach.']

    for prefix in prefixes:
        pos = 0
        while True:
            pos = data.find(prefix, pos)
            if pos == -1:
                break

            # Find end of string (null, semicolon, or non-printable)
            end = pos
            while end < len(data) and data[end] >= 32 and data[end] < 127 and data[end] not in (0, ord(';')):
                end += 1

            if end > pos + 4:
                s = data[pos:end].decode('ascii', errors='ignore')
                strings.append((pos, s))

            pos += 1

    return strings


def extract_string_refs(data: bytes) -> list[tuple[int, int, str]]:
    """Extract string table references (id, offset, table type)."""
    refs = []

    # Pattern: <id1> 01 06 07 str.qst CC <field>
    marker = b'\x01\x06\x07str.'
    pos = 0

    while True:
        pos = data.find(marker, pos)
        if pos == -1:
            break

        if pos >= 1:
            id1 = data[pos - 1]
            # Find the string table type (qst, mpn, etc.)
            end = pos + 7
            while end < len(data) and data[end] >= 32 and data[end] < 127 and data[end] != 0xCC:
                end += 1
            table_type = data[pos+7:end].decode('ascii', errors='ignore')
            refs.append((id1, pos, table_type))

        pos += 1

    return refs


def find_fqn_end(payload: bytes) -> int | None:
    """Find where the FQN ends in the payload.

    The FQN is a length-prefixed string starting with 'qst.', 'npc.', etc.
    Returns the offset of the first byte after the FQN.
    """
    # Find the FQN start by looking for common prefixes
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.']:
        pos = payload.find(prefix)
        if pos > 0 and pos < 30:
            # The length byte is right before the prefix
            fqn_len = payload[pos - 1]
            fqn_end = pos + fqn_len - 1  # -1 because length includes the length byte
            if fqn_end < len(payload):
                return fqn_end
    return None


def extract_string_table_id(payload: bytes) -> int | None:
    """Extract the string table id2 from the GOM header.

    Located after the FQN, encoded as:
    - CC <u32 LE> for values < 256
    - CD <u16 BE> for values >= 256

    The pattern after FQN is: CA <4b> <1b> CA <4b> [CC/CD <id2>]
    """
    fqn_end = find_fqn_end(payload)
    if fqn_end is None or fqn_end + 20 > len(payload):
        return None

    # After FQN: CA <4b> <1b> CA <4b> [CC/CD <id2>]
    # That's 1+4+1+1+4+1 = 12 bytes to the id2 marker
    id2_offset = fqn_end + 12

    if id2_offset + 5 > len(payload):
        return None

    marker = payload[id2_offset]
    if marker == 0xCC:  # u32 LE for values < 256
        return struct.unpack('<I', payload[id2_offset+1:id2_offset+5])[0]
    elif marker == 0xCD:  # u16 BE for values >= 256
        return struct.unpack('>H', payload[id2_offset+1:id2_offset+3])[0]

    return None


def analyze_quest(payload: bytes, fqn: str, conn: sqlite3.Connection) -> dict:
    """Analyze a quest payload and extract key information."""
    # Extract the string table id2 from header
    string_table_id = extract_string_table_id(payload)

    result = {
        'fqn': fqn,
        'size': len(payload),
        'string_table_id': string_table_id,
        'strings': [],
        'string_refs': [],
        'fqn_refs': [],
        'field_ids': defaultdict(int),
        'guid_refs': [],
    }

    # Skip header (16 bytes + FQN)
    fqn_len = payload[16] if len(payload) > 16 else 0
    data_start = 17 + fqn_len

    # Decode structured data
    for val in decode_all(payload, data_start):
        if val.marker == 0xCC:  # Field ID
            result['field_ids'][val.value] += 1
        elif val.marker == 0xCF:  # u64/GUID ref
            # Check if this matches an object GUID
            guid_hex = f'{val.value:016X}'
            guid_row = conn.execute(
                'SELECT fqn, kind FROM objects WHERE guid = ? LIMIT 1',
                (guid_hex,)
            ).fetchone()
            if guid_row:
                result['guid_refs'].append((val.offset, guid_hex, guid_row[0], guid_row[1]))

    # Extract embedded strings
    result['strings'] = extract_strings(payload)

    # Extract string references
    result['string_refs'] = extract_string_refs(payload)

    # Categorize FQN references
    for offset, s in result['strings']:
        if s.startswith('qst.'):
            result['fqn_refs'].append(('quest', offset, s))
        elif s.startswith('npc.'):
            result['fqn_refs'].append(('npc', offset, s))
        elif s.startswith('mpn.'):
            result['fqn_refs'].append(('mission_phase', offset, s))
        elif s.startswith('spn.'):
            result['fqn_refs'].append(('spawn', offset, s))
        elif s.startswith('cdx.'):
            result['fqn_refs'].append(('codex', offset, s))

    return result


def print_analysis(result: dict, conn: sqlite3.Connection):
    """Print analysis results."""
    print(f"\n{'='*80}")
    print(f"Quest: {result['fqn']}")
    print(f"Payload size: {result['size']} bytes")
    print(f"String table id: {result.get('string_table_id', 'unknown')}")

    # String references with lookups using the quest's string table id
    id2 = result.get('string_table_id')
    print(f"\n--- String References ({len(result['string_refs'])}) ---")
    for id1, offset, table_type in result['string_refs'][:15]:
        if id2:
            row = conn.execute(
                'SELECT text FROM strings WHERE id1 = ? AND id2 = ? LIMIT 1',
                (id1, id2)
            ).fetchone()
            if row:
                print(f"  id1={id1:3d}: \"{row[0][:60]}\"")
            else:
                print(f"  id1={id1:3d}: NOT FOUND in id2={id2}")
        else:
            print(f"  id1={id1:3d}: no string table id")

    # FQN references
    print(f"\n--- FQN References ---")
    for cat, offset, fqn in result['fqn_refs'][:20]:
        print(f"  [{cat:12s}] {fqn[:60]}")

    # GUID references
    if result['guid_refs']:
        print(f"\n--- GUID References ({len(result['guid_refs'])}) ---")
        for offset, guid, fqn, kind in result['guid_refs'][:10]:
            print(f"  {guid}: {fqn[:50]} ({kind})")

    # Most common field IDs
    print(f"\n--- Top Field IDs ({len(result['field_ids'])} unique) ---")
    for field_id, count in sorted(result['field_ids'].items(), key=lambda x: -x[1])[:10]:
        print(f"  0x{field_id:08X}: {count:3d} occurrences")


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'

    conn = sqlite3.connect(db_path)

    # Analyze Sith Warrior Korriban quests
    quests = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE fqn LIKE 'qst.location.korriban.class.sith_warrior.%'
        ORDER BY fqn
    """).fetchall()

    print(f"Analyzing {len(quests)} Sith Warrior Korriban quests")

    for payload_b64, fqn, guid in quests:
        if not payload_b64:
            continue

        payload = base64.b64decode(payload_b64)
        result = analyze_quest(payload, fqn, conn)
        print_analysis(result, conn)

    conn.close()


if __name__ == '__main__':
    main()
