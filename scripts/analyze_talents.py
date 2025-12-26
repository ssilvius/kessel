#!/usr/bin/env python3
"""Full byte-by-byte decode of talent GOM payloads."""

import base64
import sqlite3
import struct
from pathlib import Path

DB_PATH = Path(__file__).parent.parent.parent.parent / 'data' / 'spice.7.8b.v8.sqlite'

def hexdump(data, start=0, length=None):
    """Pretty hex dump with offsets."""
    if length:
        data = data[start:start+length]
    else:
        data = data[start:]

    lines = []
    for i in range(0, len(data), 16):
        chunk = data[i:i+16]
        hex_part = ' '.join(f'{b:02x}' for b in chunk)
        ascii_part = ''.join(chr(b) if 32 <= b < 127 else '.' for b in chunk)
        lines.append(f'{start+i:04x}: {hex_part:<48} {ascii_part}')
    return '\n'.join(lines)

def decode_payload(payload, conn):
    """Decode entire payload byte by byte."""
    result = {'fields': [], 'raw_hex': payload.hex()}
    i = 0

    # Byte 0-1: Usually small values or padding
    result['fields'].append({
        'offset': 0, 'size': 2, 'name': 'prefix',
        'hex': payload[0:2].hex(),
        'value': struct.unpack_from('<H', payload, 0)[0]
    })
    i = 2

    # Look for FQN - length-prefixed string starting with 'tal.'
    fqn_start = payload.find(b'tal.')
    if fqn_start > 0:
        fqn_len = payload[fqn_start - 1]
        fqn_bytes = payload[fqn_start:fqn_start + fqn_len]
        try:
            fqn = fqn_bytes.decode('ascii')
        except:
            fqn = fqn_bytes.hex()

        # Record bytes before FQN
        result['fields'].append({
            'offset': 2, 'size': fqn_start - 1 - 2, 'name': 'pre_fqn',
            'hex': payload[2:fqn_start-1].hex()
        })

        result['fields'].append({
            'offset': fqn_start - 1, 'size': 1 + len(fqn_bytes), 'name': 'fqn',
            'len_byte': fqn_len, 'value': fqn
        })

        i = fqn_start + fqn_len

    # Now decode msgpack-style values after FQN
    values = []
    while i < len(payload):
        marker = payload[i]

        if marker == 0xc0:  # nil
            values.append({'offset': i, 'type': 'nil', 'size': 1})
            i += 1
        elif marker == 0xc2:  # false
            values.append({'offset': i, 'type': 'false', 'size': 1})
            i += 1
        elif marker == 0xc3:  # true
            values.append({'offset': i, 'type': 'true', 'size': 1})
            i += 1
        elif marker == 0xca:  # float32 or u32 LE (SWTOR uses for string refs)
            if i + 5 <= len(payload):
                val = struct.unpack_from('<I', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'ca_u32', 'size': 5, 'value': val, 'hex': payload[i:i+5].hex()})
            i += 5
        elif marker == 0xcb:  # ref32 (hash reference)
            if i + 5 <= len(payload):
                val = struct.unpack_from('<I', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'cb_ref', 'size': 5, 'value': f'{val:08X}'})
            i += 5
        elif marker == 0xcc:  # u8 extended to u32 LE (SWTOR specific)
            if i + 5 <= len(payload):
                val = struct.unpack_from('<I', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'cc_u32', 'size': 5, 'value': val, 'hex': payload[i:i+5].hex()})
            i += 5
        elif marker == 0xcd:  # u16 BE (standard msgpack)
            if i + 3 <= len(payload):
                val = struct.unpack_from('>H', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'cd_u16be', 'size': 3, 'value': val})
            i += 3
        elif marker == 0xce:  # u32 LE
            if i + 5 <= len(payload):
                val = struct.unpack_from('<I', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'ce_u32', 'size': 5, 'value': val})
            i += 5
        elif marker == 0xcf:  # u64 - SWTOR GUIDs are BIG-ENDIAN!
            if i + 9 <= len(payload):
                val = struct.unpack_from('>Q', payload, i + 1)[0]  # BIG endian
                guid = f'{val:016X}'
                # Look up GUID
                row = conn.execute('SELECT fqn, kind FROM objects WHERE guid = ?', (guid,)).fetchone()
                ref = f'{row[0]} ({row[1]})' if row else 'unknown'
                values.append({'offset': i, 'type': 'cf_guid', 'size': 9, 'guid': guid, 'ref': ref})
            i += 9
        elif marker == 0xd0:  # int8
            if i + 2 <= len(payload):
                val = struct.unpack_from('b', payload, i + 1)[0]
                values.append({'offset': i, 'type': 'd0_i8', 'size': 2, 'value': val})
            i += 2
        elif marker == 0xa0:  # empty fixstr
            values.append({'offset': i, 'type': 'fixstr_empty', 'size': 1})
            i += 1
        elif 0xa1 <= marker <= 0xbf:  # fixstr 1-31 bytes
            length = marker - 0xa0
            # Check if this looks like actual string data (printable ASCII)
            if i + 1 + length <= len(payload):
                chunk = payload[i+1:i+1+length]
                is_printable = all(32 <= b < 127 for b in chunk)
                if is_printable:
                    s = chunk.decode('ascii')
                    values.append({'offset': i, 'type': f'fixstr_{length}', 'size': 1+length, 'value': s})
                    i += 1 + length
                else:
                    # Not a string, treat as raw byte (SWTOR may not use fixstr)
                    values.append({'offset': i, 'type': 'raw', 'size': 1, 'value': marker, 'hex': f'{marker:02x}'})
                    i += 1
            else:
                i += 1
        elif 0x80 <= marker <= 0x8f:  # fixmap 0-15 elements
            count = marker - 0x80
            values.append({'offset': i, 'type': f'fixmap_{count}', 'size': 1})
            i += 1
        elif 0x90 <= marker <= 0x9f:  # fixarray 0-15 elements
            count = marker - 0x90
            values.append({'offset': i, 'type': f'fixarray_{count}', 'size': 1})
            i += 1
        elif marker <= 0x7f:  # positive fixint
            values.append({'offset': i, 'type': 'fixint', 'size': 1, 'value': marker})
            i += 1
        elif marker >= 0xe0:  # negative fixint
            values.append({'offset': i, 'type': 'negint', 'size': 1, 'value': marker - 256})
            i += 1
        else:
            # Unknown - record raw byte
            values.append({'offset': i, 'type': 'raw', 'size': 1, 'value': marker, 'hex': f'{marker:02x}'})
            i += 1

    result['values'] = values
    return result

DISCIPLINE_LEVELS = {15, 23, 27, 35, 39, 43, 47, 51, 60, 64, 68, 73, 78}

def analyze_talents(conn):
    """Analyze all corruption talents to find level/ability patterns."""
    rows = conn.execute("""
        SELECT fqn, guid, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects WHERE kind = 'tal' AND fqn LIKE '%corruption%'
        ORDER BY fqn
    """).fetchall()

    print(f"Analyzing {len(rows)} Corruption talents\n")
    print("=" * 80)

    for fqn, guid, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)
        result = decode_payload(payload, conn)

        # Extract ability refs and discipline levels
        ability_refs = []
        level_vals = []
        for val in result['values']:
            if val['type'] == 'cf_guid' and val['guid'].startswith('E000'):
                ability_refs.append((val['offset'], val['guid'], val['ref']))
            if val['type'] == 'fixint' and val['value'] in DISCIPLINE_LEVELS:
                level_vals.append((val['offset'], val['value']))

        print(f"\n{fqn}")
        print(f"  Size: {len(payload)}")

        if level_vals:
            print(f"  Discipline levels: {level_vals}")
            # Show context around first level value
            off, lvl = level_vals[0]
            start = max(0, off - 10)
            end = min(len(payload), off + 10)
            print(f"    Context @{off:04x} (level {lvl}): {payload[start:end].hex()}")

        if ability_refs:
            print(f"  Ability refs:")
            for off, guid, ref in ability_refs:
                # Show context before GUID
                start = max(0, off - 10)
                print(f"    @{off:04x}: {ref}")
                print(f"      Pre-context: {payload[start:off].hex()}")

def find_patterns(conn):
    """Find common patterns across all talents."""
    rows = conn.execute("""
        SELECT fqn, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects WHERE kind = 'tal'
        ORDER BY fqn
    """).fetchall()

    print(f"\nAnalyzing {len(rows)} talents for patterns\n")

    # Pattern: ability ref marker
    ability_marker = bytes.fromhex('cc0b84e217d001cf')
    ability_marker_count = 0

    # Pattern: level in effect block (include CF marker)
    # cf 40 00 00 40 d9 54 fb 02 05 [LEVEL]
    # Full pattern is 10 bytes: cf + 8-byte guid ending in fb02 + 05
    effect_pattern = bytes.fromhex('cf40000040d954fb0205')
    print(f"Effect pattern: {effect_pattern.hex()} ({len(effect_pattern)} bytes)")
    level_contexts = []

    for fqn, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)

        # Count ability markers
        ability_marker_count += payload.count(ability_marker)

        # Find effect patterns and extract the level byte
        pos = 0
        while True:
            pos = payload.find(effect_pattern, pos)
            if pos == -1:
                break
            # Pattern now includes the 05: cf40000040d954fb0205 = 10 bytes
            # LEVEL is immediately after, at pos+10
            level_offset = pos + 10
            if level_offset < len(payload):
                level_byte = payload[level_offset]
                if level_byte in DISCIPLINE_LEVELS:
                    level_contexts.append((fqn, level_offset, level_byte))
            pos += 1

    print(f"Ability ref marker (cc0b84e217d001cf) found: {ability_marker_count} times")
    print(f"\nLevel values after effect pattern (40000040d954fb):")

    # Group by level
    from collections import defaultdict
    by_level = defaultdict(list)
    for fqn, off, lvl in level_contexts:
        by_level[lvl].append(fqn)

    for lvl in sorted(by_level.keys()):
        print(f"  Level {lvl}: {len(by_level[lvl])} talents")

    # Also check what ALL values appear at that offset
    all_values = defaultdict(int)
    for fqn, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)
        pos = 0
        while True:
            pos = payload.find(effect_pattern, pos)
            if pos == -1:
                break
            level_offset = pos + 10  # Match the pattern length
            if level_offset < len(payload):
                all_values[payload[level_offset]] += 1
            pos += 1

    print(f"\nAll values at effect pattern offset (top 20):")
    for val, count in sorted(all_values.items(), key=lambda x: -x[1])[:20]:
        marker = " <- DISCIPLINE" if val in DISCIPLINE_LEVELS else ""
        print(f"  {val:3d} (0x{val:02x}): {count} occurrences{marker}")

def deep_analysis(conn):
    """Deep analysis to find exact level and tier position."""
    from collections import defaultdict

    rows = conn.execute("""
        SELECT fqn, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects WHERE kind = 'tal'
        ORDER BY fqn
    """).fetchall()

    print("\n" + "=" * 80)
    print("DEEP ANALYSIS: PATTERN d954fb0205[LEVEL]")
    print("=" * 80)

    # Pattern: d954fb 02 05 [LEVEL] - the type ID followed by 02 05
    short_pattern = bytes.fromhex('d954fb0205')
    level_values = defaultdict(list)

    for fqn, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)

        pos = 0
        while True:
            pos = payload.find(short_pattern, pos)
            if pos == -1:
                break
            level_offset = pos + len(short_pattern)  # byte right after pattern
            if level_offset < len(payload):
                val = payload[level_offset]
                level_values[val].append((fqn, pos, level_offset))
            pos += 1

    print("\nValues found after d954fb0205 pattern:")
    for val in sorted(level_values.keys()):
        count = len(level_values[val])
        is_level = " <-- DISCIPLINE" if val in DISCIPLINE_LEVELS else ""
        print(f"  {val:3d} (0x{val:02x}): {count:4d} occurrences{is_level}")

    # Show sample talents for each discipline level
    print("\n" + "=" * 80)
    print("SAMPLE TALENTS BY DISCIPLINE LEVEL")
    print("=" * 80)

    for level in sorted(DISCIPLINE_LEVELS):
        talents = level_values.get(level, [])
        if talents:
            print(f"\nLevel {level}: {len(talents)} occurrences")
            for fqn, pat_pos, lvl_pos in talents[:3]:
                print(f"    {fqn}")

    # Find the FIRST occurrence of level in each talent
    print("\n" + "=" * 80)
    print("FIRST d954fb0205[LEVEL] IN EACH TALENT")
    print("=" * 80)

    first_level_per_talent = {}
    for fqn, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)

        pos = payload.find(short_pattern)
        if pos != -1:
            level_offset = pos + len(short_pattern)
            if level_offset < len(payload):
                val = payload[level_offset]
                if val in DISCIPLINE_LEVELS:
                    first_level_per_talent[fqn] = val

    print(f"\nTalents with discipline level at first d954fb0205 pattern: {len(first_level_per_talent)}")

    level_dist = defaultdict(int)
    for fqn, lvl in first_level_per_talent.items():
        level_dist[lvl] += 1

    print("\nDistribution of first-found levels:")
    for lvl in sorted(level_dist.keys()):
        print(f"  Level {lvl}: {level_dist[lvl]} talents")

    # Now look for ability GUID refs
    print("\n" + "=" * 80)
    print("ABILITY GUID REFERENCES (pattern: d001cfe000)")
    print("=" * 80)

    ability_pattern = bytes.fromhex('d001cfe000')  # d0 01 cf e0 00
    ability_refs_per_talent = defaultdict(list)

    for fqn, payload_b64 in rows:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)

        pos = 0
        while True:
            pos = payload.find(ability_pattern, pos)
            if pos == -1:
                break
            # The CF marker is at pos+2, GUID starts at pos+3
            guid_start = pos + 3
            if guid_start + 8 <= len(payload):
                guid_bytes = payload[guid_start:guid_start+8]
                guid_hex = guid_bytes.hex().upper()
                ability_refs_per_talent[fqn].append(guid_hex)
            pos += 1

    talents_with_abilities = len([f for f, refs in ability_refs_per_talent.items() if refs])
    print(f"Talents with ability refs: {talents_with_abilities}")

    # Look up a few
    sample_fqns = list(ability_refs_per_talent.keys())[:5]
    for fqn in sample_fqns:
        refs = ability_refs_per_talent[fqn]
        print(f"\n{fqn}: {len(refs)} ability refs")
        for guid in refs[:3]:
            row = conn.execute('SELECT fqn FROM objects WHERE guid = ?', (guid,)).fetchone()
            ref_fqn = row[0] if row else 'unknown'
            print(f"    {guid} -> {ref_fqn}")


def main():
    conn = sqlite3.connect(DB_PATH)
    analyze_talents(conn)
    find_patterns(conn)
    deep_analysis(conn)
    conn.close()

if __name__ == '__main__':
    main()
