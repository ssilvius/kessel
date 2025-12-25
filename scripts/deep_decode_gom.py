#!/usr/bin/env python3
"""Deep GOM decoder - fully parse the binary structure."""

import base64
import struct
import sqlite3
import sys
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any


@dataclass
class GomValue:
    """A decoded GOM value with context."""
    offset: int
    marker: int
    value: Any
    size: int

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
    """Decode a single value. Returns (value, next_pos)."""
    if pos >= len(data):
        return None, pos

    marker = data[pos]

    if marker <= 0x7F:
        return GomValue(pos, marker, marker, 1), pos + 1

    if marker >= 0xE0:
        return GomValue(pos, marker, marker - 256, 1), pos + 1

    if marker == 0xC0:
        return GomValue(pos, marker, None, 1), pos + 1

    if marker == 0xC2:
        return GomValue(pos, marker, False, 1), pos + 1

    if marker == 0xC3:
        return GomValue(pos, marker, True, 1), pos + 1

    if marker in (0xCA, 0xCB, 0xCC, 0xCE):
        if pos + 5 > len(data):
            return None, pos + 1
        val = struct.unpack('<I', data[pos+1:pos+5])[0]
        return GomValue(pos, marker, val, 5), pos + 5

    if marker == 0xCD:
        if pos + 3 > len(data):
            return None, pos + 1
        val = struct.unpack('>H', data[pos+1:pos+3])[0]
        return GomValue(pos, marker, val, 3), pos + 3

    if marker == 0xCF:
        if pos + 9 > len(data):
            return None, pos + 1
        val = struct.unpack('<Q', data[pos+1:pos+9])[0]
        return GomValue(pos, marker, val, 9), pos + 9

    return GomValue(pos, marker, data[pos], 1), pos + 1


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


def parse_full_structure(payload: bytes) -> list[GomValue]:
    """Parse entire payload into a list of values."""
    values = []
    bounds = find_fqn_bounds(payload)
    if not bounds:
        return values

    fqn_start, fqn_end = bounds

    # Parse from after FQN
    pos = fqn_end
    while pos < len(payload):
        val, next_pos = decode_value(payload, pos)
        if val is None:
            break
        values.append(val)
        pos = next_pos

    return values


def analyze_structure(payload: bytes, fqn: str, conn: sqlite3.Connection):
    """Deep analysis of GOM structure."""
    print(f"\n{'='*80}")
    print(f"FQN: {fqn}")
    print(f"Size: {len(payload)} bytes")

    bounds = find_fqn_bounds(payload)
    if not bounds:
        print("Could not find FQN bounds")
        return

    fqn_start, fqn_end = bounds
    print(f"FQN at: {fqn_start}-{fqn_end}")

    # Parse all values
    values = parse_full_structure(payload)
    print(f"Parsed {len(values)} values")

    # Categorize values
    type_counts = defaultdict(int)
    for v in values:
        type_counts[v.type_name] += 1

    print("\n--- Value Type Counts ---")
    for t, c in sorted(type_counts.items(), key=lambda x: -x[1]):
        print(f"  {t:10s}: {c:5d}")

    # Analyze CF (u64) patterns - potential GUID references
    cf_values = [v for v in values if v.marker == 0xCF]
    print(f"\n--- CF (u64) Analysis ({len(cf_values)} values) ---")

    # Check suffix patterns
    suffix_counts = defaultdict(int)
    for v in cf_values:
        suffix = v.value & 0xFFFFFFFF
        suffix_counts[suffix] += 1

    print("Top suffixes:")
    for suffix, count in sorted(suffix_counts.items(), key=lambda x: -x[1])[:10]:
        print(f"  0x{suffix:08X}: {count:3d}")

    # Try to match CF values against object GUIDs
    print("\nGUID matches:")
    matched = 0
    for v in cf_values[:50]:
        # Try direct match
        guid_hex = f'{v.value:016X}'
        row = conn.execute(
            'SELECT fqn, kind FROM objects WHERE guid = ? LIMIT 1',
            (guid_hex,)
        ).fetchone()
        if row:
            print(f"  {guid_hex} -> {row[0][:50]} ({row[1]})")
            matched += 1
    print(f"Matched {matched}/{min(50, len(cf_values))} CF values to GUIDs")

    # Analyze CB (ref32) patterns - potential hash references
    cb_values = [v for v in values if v.marker == 0xCB]
    print(f"\n--- CB (ref32) Analysis ({len(cb_values)} values) ---")

    cb_counts = defaultdict(int)
    for v in cb_values:
        cb_counts[v.value] += 1

    print("Most common CB values:")
    for val, count in sorted(cb_counts.items(), key=lambda x: -x[1])[:15]:
        print(f"  0x{val:08X}: {count:3d}")

    # Analyze CC (field) patterns
    cc_values = [v for v in values if v.marker == 0xCC]
    print(f"\n--- CC (field) Analysis ({len(cc_values)} values) ---")

    cc_counts = defaultdict(int)
    for v in cc_values:
        cc_counts[v.value] += 1

    print("Most common field IDs:")
    for val, count in sorted(cc_counts.items(), key=lambda x: -x[1])[:20]:
        # Try to decode field structure
        # Hypothesis: low 3 bytes = name hash, high byte = type index
        name_hash = val & 0xFFFFFF
        type_idx = (val >> 24) & 0xFF
        print(f"  0x{val:08X} (type={type_idx:3d}, hash=0x{name_hash:06X}): {count:3d}")

    # Look for patterns between CC fields and following values
    print("\n--- Field Value Patterns ---")
    analyze_field_patterns(values)


def analyze_field_patterns(values: list[GomValue]):
    """Analyze what values follow each field ID."""
    field_followers = defaultdict(lambda: defaultdict(int))

    for i, v in enumerate(values):
        if v.marker == 0xCC and i + 1 < len(values):
            next_val = values[i + 1]
            field_followers[v.value][next_val.type_name] += 1

    print("Top fields and what follows them:")
    for field_id, followers in sorted(field_followers.items(), key=lambda x: -sum(x[1].values()))[:15]:
        total = sum(followers.values())
        follower_str = ", ".join(f"{t}:{c}" for t, c in sorted(followers.items(), key=lambda x: -x[1])[:3])
        print(f"  0x{field_id:08X} ({total:3d}): {follower_str}")


def analyze_embedded_strings(payload: bytes, conn: sqlite3.Connection):
    """Analyze all embedded strings in the payload."""
    print("\n--- Embedded Strings ---")

    # Find all length-prefixed strings
    strings = []
    i = 0
    while i < len(payload) - 4:
        length = payload[i]
        if 4 < length < 150 and i + 1 + length <= len(payload):
            potential = payload[i+1:i+1+length]
            if all(32 <= b < 127 for b in potential):
                try:
                    s = potential.decode('ascii')
                    if '.' in s or '_' in s:
                        strings.append((i, length, s))
                except:
                    pass
        i += 1

    # Categorize by prefix
    by_prefix = defaultdict(list)
    for offset, length, s in strings:
        prefix = s.split('.')[0] if '.' in s else 'other'
        by_prefix[prefix].append((offset, s))

    for prefix, items in sorted(by_prefix.items()):
        print(f"\n{prefix}.* ({len(items)} strings):")
        for offset, s in items[:10]:
            # Check if exists in database
            exists = conn.execute('SELECT 1 FROM objects WHERE fqn = ? LIMIT 1', (s,)).fetchone()
            marker = '[EXISTS]' if exists else ''
            print(f"  {offset:5d}: {s[:60]} {marker}")


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Get a complex quest for analysis
    row = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn
        FROM objects
        WHERE fqn = 'qst.location.korriban.class.sith_warrior.the_final_trial'
    """).fetchone()

    if row:
        payload_b64, fqn = row
        payload = base64.b64decode(payload_b64)
        analyze_structure(payload, fqn, conn)
        analyze_embedded_strings(payload, conn)

    # Also analyze an ability and NPC for comparison
    print("\n" + "="*80)
    print("COMPARING WITH ABILITY OBJECT")
    print("="*80)

    row = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn
        FROM objects
        WHERE fqn LIKE 'abl.%' AND kind = 'Ability'
        LIMIT 1
    """).fetchone()

    if row:
        payload_b64, fqn = row
        payload = base64.b64decode(payload_b64)
        analyze_structure(payload, fqn, conn)

    conn.close()


if __name__ == '__main__':
    main()
