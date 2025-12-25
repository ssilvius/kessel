#!/usr/bin/env python3
"""Analyze quest flow and prerequisites from GOM data."""

import base64
import struct
import sqlite3
import sys
import json
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any


@dataclass
class QuestInfo:
    """Complete quest information."""
    fqn: str
    guid: str
    name: str
    string_table_id: int
    objectives: list[tuple[int, str]]  # (id1, text)
    milestone_vars: list[str]
    spawn_refs: list[str]
    npc_refs: list[str]
    phase_refs: list[str]
    codex_refs: list[str]
    guid_refs: list[str]  # Referenced object GUIDs


def find_fqn_end(payload: bytes) -> int | None:
    """Find where the FQN ends."""
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if pos > 0 and pos < 30:
            fqn_len = payload[pos - 1]
            fqn_end = pos - 1 + fqn_len
            if fqn_end < len(payload):
                return fqn_end
    return None


def extract_string_table_id(payload: bytes) -> int | None:
    """Extract the string table id2."""
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


def extract_objectives(payload: bytes, string_table_id: int, conn: sqlite3.Connection) -> list[tuple[int, str]]:
    """Extract quest objectives from string table references."""
    objectives = []
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
            if row:
                objectives.append((id1, row[0]))
        pos += 1
    return objectives


def extract_fqn_refs(payload: bytes, prefix: bytes) -> list[str]:
    """Extract FQN references."""
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
            if s not in refs:
                refs.append(s)
        pos += 1
    return refs


def extract_milestone_vars(payload: bytes) -> list[str]:
    """Extract quest milestones."""
    vars = []
    for prefix in [b'qm_', b'go_', b'has_']:
        pos = 0
        while True:
            pos = payload.find(prefix, pos)
            if pos == -1:
                break
            end = pos
            while end < len(payload) and (payload[end] == ord('_') or payload[end].to_bytes(1, 'big').isalnum()):
                end += 1
            varname = payload[pos:end].decode('ascii', errors='ignore')
            if varname and varname not in vars:
                vars.append(varname)
            pos = end
    return vars


def extract_guid_refs(payload: bytes) -> list[str]:
    """Extract GUID references (CF u64, big-endian)."""
    refs = []
    pos = 0
    while pos < len(payload) - 9:
        if payload[pos] == 0xCF:
            val = struct.unpack('>Q', payload[pos+1:pos+9])[0]
            guid_hex = f'{val:016X}'
            if guid_hex.startswith('E000') and guid_hex not in refs:
                refs.append(guid_hex)
            pos += 9
        else:
            pos += 1
    return refs


def analyze_quest(payload: bytes, fqn: str, guid: str, conn: sqlite3.Connection) -> QuestInfo:
    """Analyze a quest payload."""
    string_table_id = extract_string_table_id(payload) or 0

    # Quest name from id1=88
    name_row = conn.execute(
        'SELECT text FROM strings WHERE id1 = 88 AND id2 = ? LIMIT 1',
        (string_table_id,)
    ).fetchone()
    quest_name = name_row[0] if name_row else fqn.split('.')[-1]

    return QuestInfo(
        fqn=fqn,
        guid=guid,
        name=quest_name,
        string_table_id=string_table_id,
        objectives=extract_objectives(payload, string_table_id, conn),
        milestone_vars=extract_milestone_vars(payload),
        spawn_refs=extract_fqn_refs(payload, b'spn.'),
        npc_refs=extract_fqn_refs(payload, b'npc.'),
        phase_refs=extract_fqn_refs(payload, b'mpn.'),
        codex_refs=extract_fqn_refs(payload, b'cdx.'),
        guid_refs=extract_guid_refs(payload),
    )


def build_quest_graph(quests: list[QuestInfo], conn: sqlite3.Connection) -> dict:
    """Build a graph of quest relationships."""
    # Map GUIDs to quests
    guid_to_quest = {q.guid: q for q in quests}
    fqn_to_quest = {q.fqn: q for q in quests}

    # Find cross-references
    relationships = defaultdict(list)

    for quest in quests:
        # Check if any GUID refs point to other quests
        for guid_ref in quest.guid_refs:
            if guid_ref in guid_to_quest and guid_ref != quest.guid:
                ref_quest = guid_to_quest[guid_ref]
                relationships[quest.fqn].append({
                    'type': 'references_quest',
                    'target': ref_quest.fqn,
                    'via': 'guid'
                })

        # Check for shared NPCs (might indicate quest chain)
        for other in quests:
            if other.fqn == quest.fqn:
                continue
            shared_npcs = set(quest.npc_refs) & set(other.npc_refs)
            if shared_npcs:
                relationships[quest.fqn].append({
                    'type': 'shared_npcs',
                    'target': other.fqn,
                    'npcs': list(shared_npcs)
                })

        # Check for milestone references (has_completed_* patterns)
        for var in quest.milestone_vars:
            if var.startswith('has_'):
                # This suggests a prerequisite check
                relationships[quest.fqn].append({
                    'type': 'prerequisite_check',
                    'variable': var
                })

    return dict(relationships)


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Get all Sith Warrior origin story quests
    planet_order = ['korriban', 'dromund_kaas', 'balmorra', 'nar_shaddaa', 'tatooine', 'alderaan', 'hoth', 'quesh', 'belsavis', 'voss', 'corellia']

    all_quests = []
    for planet in planet_order:
        quests = conn.execute("""
            SELECT json_extract(json, '$.payload_b64'), fqn, guid
            FROM objects
            WHERE fqn LIKE ?
            AND kind = 'Quest'
            ORDER BY fqn
        """, (f'qst.location.{planet}.class.sith_warrior.%',)).fetchall()

        print(f"\n{'='*80}")
        print(f"Planet: {planet.upper()} ({len(quests)} quests)")
        print(f"{'='*80}")

        for payload_b64, fqn, guid in quests:
            if not payload_b64:
                continue
            payload = base64.b64decode(payload_b64)
            quest = analyze_quest(payload, fqn, guid, conn)
            all_quests.append(quest)

            print(f"\n{quest.name}")
            print(f"  FQN: {quest.fqn}")
            print(f"  GUID: {quest.guid}")

            if quest.objectives:
                print(f"  Objectives ({len(quest.objectives)}):")
                for id1, text in quest.objectives[:5]:
                    print(f"    [{id1:3d}] {text[:60]}")
                if len(quest.objectives) > 5:
                    print(f"    ... and {len(quest.objectives) - 5} more")

            if quest.milestone_vars:
                print(f"  Milestones: {', '.join(quest.milestone_vars[:5])}")
                if len(quest.milestone_vars) > 5:
                    print(f"    ... and {len(quest.milestone_vars) - 5} more")

    # Build and print quest graph
    print(f"\n{'='*80}")
    print("QUEST RELATIONSHIPS")
    print(f"{'='*80}")

    graph = build_quest_graph(all_quests, conn)
    for quest_fqn, rels in graph.items():
        quest_name = quest_fqn.split('.')[-1]
        prereqs = [r for r in rels if r['type'] == 'prerequisite_check']
        refs = [r for r in rels if r['type'] == 'references_quest']

        if prereqs or refs:
            print(f"\n{quest_name}:")
            for p in prereqs:
                print(f"  PREREQ CHECK: {p['variable']}")
            for r in refs:
                print(f"  REFERENCES: {r['target'].split('.')[-1]}")

    conn.close()


if __name__ == '__main__':
    main()
