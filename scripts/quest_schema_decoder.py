#!/usr/bin/env python3
"""Complete quest schema decoder - reverse engineer the full quest structure."""

import base64
import struct
import sqlite3
import sys
import json
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field, asdict
from typing import Any, Optional


# Known CB type hashes (reverse engineered)
CB_TYPES = {
    0xC91F4101: 'QuestVariable',
    0xBC046801: 'JournalEntry',
    0x0EF81F1F: 'GoalRef',
    0xB17E5B0D: 'StepDef',
    0x4F05B40A: 'TaskDef',
    0xBC48ADB2: 'SpawnRef',
    0xCD88C310: 'GuidRef',
    0x8F297503: 'PhaseRef',
}

# Known CC field hashes (reverse engineered)
CC_FIELDS = {
    # Quest-specific
    0x190AB20C: 'questSteps',
    0x7AE24C04: 'questFlags',
    0x73FE8534: 'questVarRef',
    0xE1FC0C01: 'spawnDef',
    0x1E8D820D: 'missionPhase',
    0x845B620D: 'npcRef',

    # Shared
    0x6F6FAE37: 'localizedText',
    0x6E968434: 'stringSlot',

    # Journal
    0xD3014605: 'journalText',
    0x311C8013: 'journalEntry',
}


@dataclass
class QuestStep:
    """A quest step/branch."""
    branch: int
    step: int
    task: int
    name: str
    objectives: list[str] = field(default_factory=list)
    milestones: list[str] = field(default_factory=list)


@dataclass
class QuestSchema:
    """Complete decoded quest structure."""
    fqn: str
    guid: str
    name: str
    string_table_id: int

    # Objectives (id1 -> text)
    objectives: dict[int, str] = field(default_factory=dict)

    # Journal entries
    journal_entries: list[str] = field(default_factory=list)

    # Quest steps (branch/step/task structure)
    steps: list[QuestStep] = field(default_factory=list)

    # Milestone variables
    milestones: list[str] = field(default_factory=list)
    prerequisites: list[str] = field(default_factory=list)

    # References
    spawn_points: list[str] = field(default_factory=list)
    npcs: list[str] = field(default_factory=list)
    mission_phases: list[str] = field(default_factory=list)
    codex_entries: list[str] = field(default_factory=list)

    # GUID references to other objects
    npc_guids: list[str] = field(default_factory=list)
    quest_guids: list[str] = field(default_factory=list)


def find_fqn_end(payload: bytes) -> Optional[int]:
    """Find where the FQN ends."""
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if 0 < pos < 30:
            fqn_len = payload[pos - 1]
            return pos - 1 + fqn_len
    return None


def extract_string_table_id(payload: bytes) -> Optional[int]:
    """Extract string table ID.

    The string table ID is encoded at FQN_end + 12 using:
    - CC: u32 little-endian (for small values)
    - CD: u16 big-endian (for values < 65536)
    - CE: 3-byte big-endian + padding (for larger values!)
    """
    fqn_end = find_fqn_end(payload)
    if fqn_end is None or fqn_end + 17 > len(payload):
        return None
    id2_offset = fqn_end + 12
    marker = payload[id2_offset]
    if marker == 0xCC:
        return struct.unpack('<I', payload[id2_offset+1:id2_offset+5])[0]
    elif marker == 0xCD:
        return struct.unpack('>H', payload[id2_offset+1:id2_offset+3])[0]
    elif marker == 0xCE:
        # CE uses 3-byte big-endian! (not 4-byte little-endian)
        return int.from_bytes(payload[id2_offset+1:id2_offset+4], 'big')
    return None


def extract_objectives(payload: bytes, stid: int, conn: sqlite3.Connection) -> dict[int, str]:
    """Extract all objective strings."""
    objectives = {}
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
                (id1, stid)
            ).fetchone()
            if row:
                objectives[id1] = row[0]
        pos += 1
    return objectives


def extract_quest_steps(payload: bytes) -> list[QuestStep]:
    """Extract quest step structure (_bX_sY_tZ patterns)."""
    steps = []
    import re

    # Find all _bX_sY_tZ patterns
    for pos in range(len(payload) - 10):
        if payload[pos:pos+2] == b'_b':
            end = pos
            while end < len(payload) and (payload[end:end+1].isalnum() or payload[end] == ord('_')):
                end += 1
            step_name = payload[pos:end].decode('ascii', errors='ignore')

            # Parse the step identifier
            match = re.match(r'_b(\d+)_s(\d+)(?:_t(\d+))?', step_name)
            if match:
                branch = int(match.group(1))
                step = int(match.group(2))
                task = int(match.group(3)) if match.group(3) else 1

                existing = next((s for s in steps if s.branch == branch and s.step == step and s.task == task), None)
                if not existing:
                    steps.append(QuestStep(
                        branch=branch,
                        step=step,
                        task=task,
                        name=step_name
                    ))

    return sorted(steps, key=lambda s: (s.branch, s.step, s.task))


def extract_milestones(payload: bytes) -> tuple[list[str], list[str]]:
    """Extract milestone variables and prerequisites."""
    milestones = []
    prerequisites = []

    for prefix in [b'qm_', b'go_']:
        pos = 0
        while True:
            pos = payload.find(prefix, pos)
            if pos == -1:
                break
            end = pos
            while end < len(payload) and (payload[end:end+1].isalnum() or payload[end] == ord('_')):
                end += 1
            varname = payload[pos:end].decode('ascii')
            if varname and varname not in milestones:
                milestones.append(varname)
            pos = end

    # has_* are prerequisites
    pos = 0
    while True:
        pos = payload.find(b'has_', pos)
        if pos == -1:
            break
        end = pos
        while end < len(payload) and (payload[end:end+1].isalnum() or payload[end] == ord('_')):
            end += 1
        varname = payload[pos:end].decode('ascii')
        if varname and varname not in prerequisites:
            prerequisites.append(varname)
        pos = end

    return milestones, prerequisites


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


def extract_guid_refs(payload: bytes, conn: sqlite3.Connection) -> tuple[list[str], list[str]]:
    """Extract GUID references, categorized by type."""
    npc_guids = []
    quest_guids = []

    pos = 0
    while pos < len(payload) - 9:
        if payload[pos] == 0xCF:
            val = struct.unpack('>Q', payload[pos+1:pos+9])[0]
            guid_hex = f'{val:016X}'
            if guid_hex.startswith('E000'):
                row = conn.execute(
                    'SELECT fqn, kind FROM objects WHERE guid = ? LIMIT 1',
                    (guid_hex,)
                ).fetchone()
                if row:
                    if row[1] == 'Npc' and guid_hex not in npc_guids:
                        npc_guids.append(guid_hex)
                    elif row[1] == 'Quest' and guid_hex not in quest_guids:
                        quest_guids.append(guid_hex)
            pos += 9
        else:
            pos += 1

    return npc_guids, quest_guids


def extract_journal_entries(payload: bytes, stid: int, conn: sqlite3.Connection) -> list[str]:
    """Extract journal entry texts."""
    entries = []

    # Journal entries are typically in id1 range 200-300
    for id1 in range(200, 300):
        row = conn.execute(
            'SELECT text FROM strings WHERE id1 = ? AND id2 = ? LIMIT 1',
            (id1, stid)
        ).fetchone()
        if row and row[0] and len(row[0]) > 20:  # Actual journal text
            entries.append(row[0])

    return entries


def decode_quest(payload: bytes, fqn: str, guid: str, conn: sqlite3.Connection) -> QuestSchema:
    """Decode a complete quest structure."""
    stid = extract_string_table_id(payload) or 0

    # Get name
    name_row = conn.execute(
        'SELECT text FROM strings WHERE id1 = 88 AND id2 = ? LIMIT 1',
        (stid,)
    ).fetchone()
    name = name_row[0] if name_row else fqn.split('.')[-1]

    # Extract all components
    objectives = extract_objectives(payload, stid, conn)
    steps = extract_quest_steps(payload)
    milestones, prerequisites = extract_milestones(payload)
    npc_guids, quest_guids = extract_guid_refs(payload, conn)
    journal = extract_journal_entries(payload, stid, conn)

    return QuestSchema(
        fqn=fqn,
        guid=guid,
        name=name,
        string_table_id=stid,
        objectives=objectives,
        journal_entries=journal,
        steps=steps,
        milestones=milestones,
        prerequisites=prerequisites,
        spawn_points=extract_fqn_refs(payload, b'spn.'),
        npcs=extract_fqn_refs(payload, b'npc.'),
        mission_phases=extract_fqn_refs(payload, b'mpn.'),
        codex_entries=extract_fqn_refs(payload, b'cdx.'),
        npc_guids=npc_guids,
        quest_guids=quest_guids,
    )


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Decode all Sith Warrior origin quests
    quests = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE fqn LIKE 'qst.location.%.class.sith_warrior.%'
        AND kind = 'Quest'
        AND json_extract(json, '$.payload_b64') IS NOT NULL
        ORDER BY fqn
    """).fetchall()

    print(f"Decoding {len(quests)} Sith Warrior quests...")

    decoded_quests = []
    for payload_b64, fqn, guid in quests:
        payload = base64.b64decode(payload_b64)
        quest = decode_quest(payload, fqn, guid, conn)
        decoded_quests.append(quest)

    # Print summary
    print(f"\n{'='*80}")
    print("DECODED QUEST SUMMARY")
    print(f"{'='*80}")

    for quest in decoded_quests:
        print(f"\n{quest.name}")
        print(f"  FQN: {quest.fqn}")
        print(f"  GUID: {quest.guid}")
        print(f"  String Table: {quest.string_table_id}")
        print(f"  Objectives: {len(quest.objectives)}")
        print(f"  Steps: {len(quest.steps)} ({', '.join(s.name for s in quest.steps[:3])}{'...' if len(quest.steps) > 3 else ''})")
        print(f"  Milestones: {len(quest.milestones)}")
        if quest.prerequisites:
            print(f"  Prerequisites: {quest.prerequisites}")
        print(f"  NPCs: {len(quest.npcs)}, Spawn Points: {len(quest.spawn_points)}")
        print(f"  NPC GUIDs: {len(quest.npc_guids)}, Quest GUIDs: {len(quest.quest_guids)}")

    # Export as JSON
    output_path = Path('quest_schema_export.json')
    export_data = []
    for quest in decoded_quests:
        q_dict = {
            'fqn': quest.fqn,
            'guid': quest.guid,
            'name': quest.name,
            'string_table_id': quest.string_table_id,
            'objectives': quest.objectives,
            'journal_entries': quest.journal_entries,
            'steps': [{'branch': s.branch, 'step': s.step, 'task': s.task, 'name': s.name} for s in quest.steps],
            'milestones': quest.milestones,
            'prerequisites': quest.prerequisites,
            'spawn_points': quest.spawn_points,
            'npcs': quest.npcs,
            'mission_phases': quest.mission_phases,
            'codex_entries': quest.codex_entries,
            'npc_guids': quest.npc_guids,
            'quest_guids': quest.quest_guids,
        }
        export_data.append(q_dict)

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\n\nExported to {output_path}")

    conn.close()


if __name__ == '__main__':
    main()
