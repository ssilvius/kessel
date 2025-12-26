#!/usr/bin/env python3
"""Analyze talent string_id linking issue."""

import sqlite3
import base64
import sys

DB_PATH = "data/spice.7.8b.v10.sqlite"

def find_all_ce_values(payload, min_val=100000, max_val=1200000):
    """Find all CE values in valid range anywhere in payload."""
    results = []
    for i in range(len(payload) - 5):
        if payload[i] == 0xCE:
            ce_bytes = payload[i + 1:i + 5]
            ce_value = int.from_bytes(ce_bytes, 'little')
            if min_val <= ce_value <= max_val:
                results.append((i, ce_value))
    return results

def main():
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    cur = conn.cursor()

    # Get string table stats
    cur.execute("SELECT MIN(id2), MAX(id2), COUNT(DISTINCT id2) FROM strings")
    row = cur.fetchone()
    print(f"Strings table: id2 range {row[0]} - {row[1]}, {row[2]} distinct values")

    # Get talent string stats
    cur.execute("SELECT MIN(id2), MAX(id2), COUNT(*) FROM strings WHERE fqn LIKE 'str.tal%'")
    row = cur.fetchone()
    print(f"Talent strings: id2 range {row[0]} - {row[1]}, {row[2]} entries")

    # Count talents with/without string_id
    cur.execute("SELECT COUNT(*) FROM objects WHERE kind='Talent' AND string_id IS NOT NULL")
    with_sid = cur.fetchone()[0]
    cur.execute("SELECT COUNT(*) FROM objects WHERE kind='Talent' AND string_id IS NULL")
    without_sid = cur.fetchone()[0]
    print(f"\nTalents: {with_sid} with string_id, {without_sid} without")

    # Search for ANY CE value in valid range for talents without string_id
    print("\nSearching for CE values in talent range (745000-1100000) in missing talents:")
    cur.execute("""
        SELECT fqn, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects
        WHERE kind='Talent' AND string_id IS NULL
        LIMIT 20
    """)

    found_count = 0
    for row in cur.fetchall():
        fqn = row['fqn']
        payload = base64.b64decode(row['payload_b64'])

        # Search for CE values in talent string range
        ce_values = find_all_ce_values(payload, min_val=745000, max_val=1100000)
        if ce_values:
            found_count += 1
            print(f"\n  {fqn}:")
            for offset, val in ce_values:
                # Check if this value exists in strings table
                cur.execute("SELECT fqn, text FROM strings WHERE id2 = ?", (val,))
                string_row = cur.fetchone()
                if string_row:
                    print(f"    offset {offset}: CE {val} -> {string_row['text'][:40]}...")
                else:
                    print(f"    offset {offset}: CE {val} (no string match)")

    print(f"\nFound {found_count}/20 talents with CE values in talent range")

    # Look at str.tal references embedded in payloads
    print("\n\nExtracting str.tal references from payloads:")
    cur.execute("""
        SELECT fqn, json_extract(json, '$.strings') as strings
        FROM objects
        WHERE kind='Talent' AND string_id IS NULL
        LIMIT 10
    """)

    import json
    for row in cur.fetchall():
        fqn = row['fqn']
        strings_json = row['strings']
        if strings_json:
            strings = json.loads(strings_json)
            str_tal_refs = [s for s in strings if s.startswith('str.tal')]
            if str_tal_refs:
                print(f"\n  {fqn}:")
                for ref in str_tal_refs[:3]:
                    print(f"    -> {ref}")
                    # Try to find this in strings table
                    cur.execute("SELECT id2, text FROM strings WHERE fqn = ?", (ref,))
                    match = cur.fetchone()
                    if match:
                        print(f"       id2={match['id2']}: {match['text'][:40]}...")

    # Check what the str.tal FQN format actually looks like
    print("\n\nStr.tal FQN format samples:")
    cur.execute("SELECT DISTINCT fqn FROM strings WHERE fqn LIKE 'str.tal%' ORDER BY fqn LIMIT 20")
    for row in cur.fetchall():
        print(f"  {row['fqn']}")

    # Look at raw bytes around str.tal to understand the binary encoding
    print("\n\nAnalyzing binary structure around str.tal references:")
    cur.execute("""
        SELECT fqn, string_id, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects
        WHERE kind='Talent' AND string_id IS NOT NULL
        LIMIT 3
    """)

    STR_TAL = b"str.tal"

    for row in cur.fetchall():
        fqn = row['fqn']
        string_id = row['string_id']
        payload = base64.b64decode(row['payload_b64'])

        print(f"\n  {fqn} (string_id={string_id}):")

        # Find str.tal references
        idx = 0
        while True:
            idx = payload.find(STR_TAL, idx)
            if idx == -1:
                break
            # Show context: 10 bytes before and 20 bytes after
            start = max(0, idx - 10)
            end = min(len(payload), idx + len(STR_TAL) + 20)
            context = payload[start:end]
            print(f"    offset {idx}: {context.hex()}")
            print(f"    ASCII: {repr(context)}")
            idx += 1

    # Now check a talent WITHOUT string_id
    print("\n\nSame for talent WITHOUT string_id:")
    cur.execute("""
        SELECT fqn, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects
        WHERE kind='Talent' AND string_id IS NULL
        LIMIT 2
    """)

    for row in cur.fetchall():
        fqn = row['fqn']
        payload = base64.b64decode(row['payload_b64'])

        print(f"\n  {fqn}:")

        idx = 0
        while True:
            idx = payload.find(STR_TAL, idx)
            if idx == -1:
                break
            start = max(0, idx - 10)
            end = min(len(payload), idx + len(STR_TAL) + 20)
            context = payload[start:end]
            print(f"    offset {idx}: {context.hex()}")
            print(f"    ASCII: {repr(context)}")
            idx += 1

    # Extract ID from str.tal pattern - 10 bytes before str.tal
    print("\n\n=== SOLUTION: Extract string_id from bytes BEFORE str.tal ===")
    print("Pattern: <4 bytes LE id> 00 00 XX XX 06 07 str.tal")

    cur.execute("""
        SELECT fqn, string_id, json_extract(json, '$.payload_b64') as payload_b64
        FROM objects
        WHERE kind='Talent'
        LIMIT 30
    """)

    matches = 0
    mismatches = 0
    new_ids_found = 0

    for row in cur.fetchall():
        fqn = row['fqn']
        existing_string_id = row['string_id']
        payload = base64.b64decode(row['payload_b64'])

        # Find first str.tal reference
        idx = payload.find(STR_TAL)
        if idx >= 10:
            # ID is 10 bytes before str.tal (4 bytes at offset -10)
            id_offset = idx - 10
            id_bytes = payload[id_offset:id_offset + 4]
            extracted_id = int.from_bytes(id_bytes, 'little')

            # Check if in valid string range
            in_range = 100000 <= extracted_id <= 1200000

            if existing_string_id:
                if existing_string_id == extracted_id:
                    matches += 1
                else:
                    mismatches += 1
                    print(f"  MISMATCH {fqn}: existing={existing_string_id}, extracted={extracted_id}")
            else:
                # No existing string_id
                if in_range:
                    # Check if this ID exists in strings table
                    cur.execute("SELECT text FROM strings WHERE id2 = ?", (extracted_id,))
                    string_match = cur.fetchone()
                    if string_match:
                        new_ids_found += 1
                        if new_ids_found <= 10:
                            print(f"  NEW MATCH {fqn}: id={extracted_id} -> {string_match['text'][:40]}...")

    print(f"\nExisting string_id matches extracted: {matches}")
    print(f"Mismatches: {mismatches}")
    print(f"New IDs found (talents without string_id): {new_ids_found}")

    # Compare talent vs ability string_id coverage
    print("\n\n=== String_id coverage by object type ===")
    cur.execute("""
        SELECT kind,
               COUNT(*) as total,
               SUM(CASE WHEN string_id IS NOT NULL THEN 1 ELSE 0 END) as with_string_id,
               ROUND(100.0 * SUM(CASE WHEN string_id IS NOT NULL THEN 1 ELSE 0 END) / COUNT(*), 1) as pct
        FROM objects
        WHERE kind IN ('Talent', 'Ability', 'Item', 'Quest', 'Npc')
        GROUP BY kind
        ORDER BY pct DESC
    """)

    print(f"{'Kind':<12} {'Total':>8} {'w/StringID':>12} {'%':>8}")
    print("-" * 44)
    for row in cur.fetchall():
        print(f"{row['kind']:<12} {row['total']:>8} {row['with_string_id']:>12} {row['pct']:>7.1f}%")

    # For talents - can we link via FQN structure?
    print("\n\n=== Alternative: Link talents to abilities they modify ===")
    print("Talents reference abilities in their payload. We can find ability names instead.")

    cur.execute("""
        SELECT fqn, json_extract(json, '$.strings') as strings
        FROM objects
        WHERE kind='Talent' AND string_id IS NULL
        LIMIT 5
    """)

    for row in cur.fetchall():
        fqn = row['fqn']
        strings_json = row['strings']
        if strings_json:
            strings = json.loads(strings_json)
            abl_refs = [s for s in strings if s.startswith('abl.')]
            if abl_refs:
                print(f"\n  {fqn}:")
                for abl in abl_refs[:3]:
                    # Look up this ability
                    cur.execute("SELECT string_id FROM objects WHERE fqn = ?", (abl,))
                    abl_row = cur.fetchone()
                    if abl_row and abl_row['string_id']:
                        cur.execute("SELECT text FROM strings WHERE id2 = ?", (abl_row['string_id'],))
                        name_row = cur.fetchone()
                        if name_row:
                            print(f"    -> {abl} = {name_row['text'][:40]}")
                        else:
                            print(f"    -> {abl} (string_id={abl_row['string_id']}, no match)")
                    else:
                        print(f"    -> {abl} (no string_id)")

    conn.close()

if __name__ == "__main__":
    main()
