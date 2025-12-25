#!/usr/bin/env python3
"""Extract complete quest structure from GOM payload."""

import base64
import struct
import sqlite3
import sys
import re
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any


@dataclass
class QuestObjective:
    """A quest objective with associated data."""
    offset: int
    string_id: int
    text: str
    milestone_var: str | None = None
    spawn_ref: str | None = None
    npc_ref: str | None = None
    phase_ref: str | None = None


@dataclass
class QuestData:
    """Extracted quest data."""
    fqn: str
    name: str
    string_table_id: int
    objectives: list[QuestObjective] = field(default_factory=list)
    milestone_vars: list[tuple[int, str]] = field(default_factory=list)
    spawn_refs: list[tuple[int, str]] = field(default_factory=list)
    npc_refs: list[tuple[int, str]] = field(default_factory=list)
    phase_refs: list[tuple[int, str]] = field(default_factory=list)
    codex_refs: list[tuple[int, str]] = field(default_factory=list)


def find_fqn_end(payload: bytes) -> int | None:
    """Find where the FQN ends in the payload."""
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if pos > 0 and pos < 30:
            fqn_len = payload[pos - 1]
            fqn_end = pos - 1 + fqn_len
            if fqn_end < len(payload):
                return fqn_end
    return None


def extract_string_table_id(payload: bytes) -> int | None:
    """Extract the string table id2 from the GOM header."""
    fqn_end = find_fqn_end(payload)
    if fqn_end is None or fqn_end + 20 > len(payload):
        return None

    id2_offset = fqn_end + 12
    if id2_offset + 5 > len(payload):
        return None

    marker = payload[id2_offset]
    if marker == 0xCC:
        return struct.unpack('<I', payload[id2_offset+1:id2_offset+5])[0]
    elif marker == 0xCD:
        return struct.unpack('>H', payload[id2_offset+1:id2_offset+3])[0]
    return None


def extract_string_refs(payload: bytes, string_table_id: int, conn: sqlite3.Connection) -> list[tuple[int, int, str]]:
    """Extract all string table references."""
    refs = []
    marker = b'\x01\x06\x07str.qst'
    pos = 0

    while True:
        pos = payload.find(marker, pos)
        if pos == -1:
            break
        if pos >= 1:
            id1 = payload[pos - 1]
            row = conn.execute(
                'SELECT text FROM strings WHERE id1 = ? AND id2 = ? LIMIT 1',
                (id1, string_table_id)
            ).fetchone()
            text = row[0] if row else None
            refs.append((pos, id1, text))
        pos += 1

    return refs


def extract_fqn_refs(payload: bytes, prefix: bytes) -> list[tuple[int, str]]:
    """Extract FQN references with a specific prefix."""
    refs = []
    pos = 0
    while True:
        pos = payload.find(prefix, pos)
        if pos == -1:
            break
        end = pos
        while end < len(payload) and 32 <= payload[end] < 127 and payload[end] != ord(';'):
            end += 1
        if end > pos + 4:
            s = payload[pos:end].decode('ascii', errors='ignore')
            refs.append((pos, s))
        pos += 1
    return refs


def extract_milestone_vars(payload: bytes) -> list[tuple[int, str]]:
    """Extract quest milestone variables (qm_*, go_*, has_*)."""
    vars = []
    for prefix in [b'qm_', b'go_', b'has_']:
        pos = 0
        while True:
            pos = payload.find(prefix, pos)
            if pos == -1:
                break
            end = pos
            while end < len(payload) and 32 <= payload[end] < 127:
                end += 1
            varname = payload[pos:end].decode('ascii')
            vars.append((pos, varname))
            pos = end
    return vars


def analyze_quest(payload: bytes, fqn: str, conn: sqlite3.Connection) -> QuestData:
    """Analyze a quest payload and extract all structured data."""
    string_table_id = extract_string_table_id(payload) or 0

    # Look up quest name (id1=88)
    name_row = conn.execute(
        'SELECT text FROM strings WHERE id1 = 88 AND id2 = ? LIMIT 1',
        (string_table_id,)
    ).fetchone()
    quest_name = name_row[0] if name_row else fqn

    data = QuestData(
        fqn=fqn,
        name=quest_name,
        string_table_id=string_table_id,
    )

    # Extract all references
    data.milestone_vars = extract_milestone_vars(payload)
    data.spawn_refs = extract_fqn_refs(payload, b'spn.')
    data.npc_refs = extract_fqn_refs(payload, b'npc.')
    data.phase_refs = extract_fqn_refs(payload, b'mpn.')
    data.codex_refs = extract_fqn_refs(payload, b'cdx.')

    # Extract string references
    string_refs = extract_string_refs(payload, string_table_id, conn)
    for pos, id1, text in string_refs:
        if text:
            obj = QuestObjective(offset=pos, string_id=id1, text=text)
            data.objectives.append(obj)

    return data


def print_quest_data(data: QuestData):
    """Print extracted quest data."""
    print(f"\n{'='*80}")
    print(f"Quest: {data.name}")
    print(f"FQN: {data.fqn}")
    print(f"String Table ID: {data.string_table_id}")

    print(f"\n--- Objectives ({len(data.objectives)}) ---")
    for obj in data.objectives:
        print(f"  [{obj.offset:5d}] id1={obj.string_id:3d}: {obj.text}")

    print(f"\n--- Milestone Variables ({len(data.milestone_vars)}) ---")
    for offset, var in data.milestone_vars:
        print(f"  [{offset:5d}] {var}")

    print(f"\n--- Spawn Points ({len(data.spawn_refs)}) ---")
    seen = set()
    for offset, ref in data.spawn_refs:
        if ref not in seen:
            print(f"  [{offset:5d}] {ref}")
            seen.add(ref)

    print(f"\n--- NPCs ({len(data.npc_refs)}) ---")
    seen = set()
    for offset, ref in data.npc_refs:
        if ref not in seen:
            print(f"  [{offset:5d}] {ref}")
            seen.add(ref)

    print(f"\n--- Mission Phases ({len(data.phase_refs)}) ---")
    seen = set()
    for offset, ref in data.phase_refs:
        if ref not in seen:
            print(f"  [{offset:5d}] {ref}")
            seen.add(ref)

    if data.codex_refs:
        print(f"\n--- Codex Entries ({len(data.codex_refs)}) ---")
        seen = set()
        for offset, ref in data.codex_refs:
            if ref not in seen:
                print(f"  [{offset:5d}] {ref}")
                seen.add(ref)


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Get Sith Warrior Korriban quests
    quests = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn
        FROM objects
        WHERE fqn LIKE 'qst.location.korriban.class.sith_warrior.%'
        ORDER BY fqn
    """).fetchall()

    print(f"Analyzing {len(quests)} Sith Warrior Korriban quests")

    for payload_b64, fqn in quests:
        if not payload_b64:
            continue
        payload = base64.b64decode(payload_b64)
        data = analyze_quest(payload, fqn, conn)
        print_quest_data(data)

    conn.close()


if __name__ == '__main__':
    main()
