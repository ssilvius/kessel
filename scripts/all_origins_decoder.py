#!/usr/bin/env python3
"""Decode all origin story quests from all 8 classes."""

import base64
import struct
import sqlite3
import sys
import json
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Optional

# Class code to display name mapping
CLASS_NAMES = {
    'sith_warrior': 'Sith Warrior',
    'sith_sorcerer': 'Sith Inquisitor',
    'bounty_hunter': 'Bounty Hunter',
    'spy': 'Imperial Agent',
    'jedi_knight': 'Jedi Knight',
    'jedi_wizard': 'Jedi Consular',
    'jedi_consular': 'Jedi Consular',  # Alias
    'smuggler': 'Smuggler',
    'trooper': 'Trooper',
}

# Faction mapping
FACTION = {
    'sith_warrior': 'Empire',
    'sith_sorcerer': 'Empire',
    'bounty_hunter': 'Empire',
    'spy': 'Empire',
    'jedi_knight': 'Republic',
    'jedi_wizard': 'Republic',
    'jedi_consular': 'Republic',
    'smuggler': 'Republic',
    'trooper': 'Republic',
}

# Planet order for quest chains
PLANET_ORDER = {
    'Empire': ['korriban', 'hutta', 'dromund_kaas', 'balmorra', 'nar_shaddaa',
               'tatooine', 'alderaan', 'taris_imperial', 'hoth', 'quesh',
               'belsavis', 'voss', 'corellia', 'ilum'],
    'Republic': ['tython', 'ord_mantell', 'coruscant', 'taris', 'nar_shaddaa',
                 'tatooine', 'alderaan', 'balmorra_republic', 'hoth', 'quesh',
                 'belsavis', 'voss', 'corellia', 'ilum'],
}

# Starting planets per class
STARTING_PLANET = {
    'sith_warrior': 'korriban',
    'sith_sorcerer': 'korriban',
    'bounty_hunter': 'hutta',
    'spy': 'hutta',
    'jedi_knight': 'tython',
    'jedi_wizard': 'tython',
    'jedi_consular': 'tython',
    'smuggler': 'ord_mantell',
    'trooper': 'ord_mantell',
}


@dataclass
class QuestStep:
    """A quest step/branch."""
    branch: int
    step: int
    task: int
    name: str


@dataclass
class Quest:
    """Complete decoded quest."""
    fqn: str
    guid: str
    name: str
    class_code: str
    class_name: str
    faction: str
    planet: str
    string_table_id: int
    objectives: dict  # id1 -> text
    journal_entries: list
    steps: list
    milestones: list
    prerequisites: list
    spawn_points: list
    npcs: list
    mission_phases: list
    codex_entries: list
    guid_refs: list  # All referenced GUIDs


def find_fqn_end(payload: bytes) -> Optional[int]:
    """Find where the FQN ends."""
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if 0 < pos < 30:
            fqn_len = payload[pos - 1]
            return pos - 1 + fqn_len
    return None


def extract_string_table_id(payload: bytes) -> Optional[int]:
    """Extract string table ID with correct endianness."""
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
        # CE uses 3-byte big-endian!
        return int.from_bytes(payload[id2_offset+1:id2_offset+4], 'big')
    return None


def extract_objectives(payload: bytes, stid: int, conn: sqlite3.Connection) -> dict:
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


def extract_quest_steps(payload: bytes) -> list:
    """Extract quest step structure (_bX_sY_tZ patterns)."""
    import re
    steps = []
    seen = set()

    for pos in range(len(payload) - 10):
        if payload[pos:pos+2] == b'_b':
            end = pos
            while end < len(payload) and (payload[end:end+1].isalnum() or payload[end] == ord('_')):
                end += 1
            step_name = payload[pos:end].decode('ascii', errors='ignore')

            match = re.match(r'_b(\d+)_s(\d+)(?:_t(\d+))?', step_name)
            if match:
                branch = int(match.group(1))
                step = int(match.group(2))
                task = int(match.group(3)) if match.group(3) else 1
                key = (branch, step, task)
                if key not in seen:
                    seen.add(key)
                    steps.append(QuestStep(branch, step, task, step_name))

    return sorted(steps, key=lambda s: (s.branch, s.step, s.task))


def extract_milestones(payload: bytes) -> tuple:
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
            varname = payload[pos:end].decode('ascii', errors='ignore')
            if varname and varname not in milestones:
                milestones.append(varname)
            pos = end

    pos = 0
    while True:
        pos = payload.find(b'has_', pos)
        if pos == -1:
            break
        end = pos
        while end < len(payload) and (payload[end:end+1].isalnum() or payload[end] == ord('_')):
            end += 1
        varname = payload[pos:end].decode('ascii', errors='ignore')
        if varname and varname not in prerequisites:
            prerequisites.append(varname)
        pos = end

    return milestones, prerequisites


def extract_fqn_refs(payload: bytes, prefix: bytes) -> list:
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


def extract_guid_refs(payload: bytes) -> list:
    """Extract all GUID references (E000... format)."""
    guids = []
    pos = 0
    while pos < len(payload) - 9:
        if payload[pos] == 0xCF:
            val = struct.unpack('>Q', payload[pos+1:pos+9])[0]
            guid_hex = f'{val:016X}'
            if guid_hex.startswith('E000') and guid_hex not in guids:
                guids.append(guid_hex)
            pos += 9
        else:
            pos += 1
    return guids


def extract_journal_entries(payload: bytes, stid: int, conn: sqlite3.Connection) -> list:
    """Extract journal entry texts."""
    entries = []
    for id1 in range(200, 300):
        row = conn.execute(
            'SELECT text FROM strings WHERE id1 = ? AND id2 = ? LIMIT 1',
            (id1, stid)
        ).fetchone()
        if row and row[0] and len(row[0]) > 20:
            entries.append(row[0])
    return entries


def extract_class_and_planet(fqn: str) -> tuple:
    """Extract class code and planet from FQN."""
    # Pattern: qst.location.{planet}.class.{class}.{quest_name}
    parts = fqn.split('.')
    if len(parts) >= 5 and parts[0] == 'qst' and parts[1] == 'location' and parts[3] == 'class':
        planet = parts[2]
        class_code = parts[4]
        return class_code, planet
    return None, None


def decode_quest(payload: bytes, fqn: str, guid: str, conn: sqlite3.Connection) -> Optional[Quest]:
    """Decode a complete quest structure."""
    class_code, planet = extract_class_and_planet(fqn)
    if not class_code or class_code not in CLASS_NAMES:
        return None

    stid = extract_string_table_id(payload) or 0

    # Get name
    name_row = conn.execute(
        'SELECT text FROM strings WHERE id1 = 88 AND id2 = ? LIMIT 1',
        (stid,)
    ).fetchone()
    name = name_row[0] if name_row else fqn.split('.')[-1]

    # Extract components
    objectives = extract_objectives(payload, stid, conn)
    steps = extract_quest_steps(payload)
    milestones, prerequisites = extract_milestones(payload)
    guid_refs = extract_guid_refs(payload)
    journal = extract_journal_entries(payload, stid, conn)

    return Quest(
        fqn=fqn,
        guid=guid,
        name=name,
        class_code=class_code,
        class_name=CLASS_NAMES[class_code],
        faction=FACTION[class_code],
        planet=planet,
        string_table_id=stid,
        objectives=objectives,
        journal_entries=journal,
        steps=[{'branch': s.branch, 'step': s.step, 'task': s.task, 'name': s.name} for s in steps],
        milestones=milestones,
        prerequisites=prerequisites,
        spawn_points=extract_fqn_refs(payload, b'spn.'),
        npcs=extract_fqn_refs(payload, b'npc.'),
        mission_phases=extract_fqn_refs(payload, b'mpn.'),
        codex_entries=extract_fqn_refs(payload, b'cdx.'),
        guid_refs=guid_refs,
    )


def extract_objectives_fast(payload: bytes, stid: int, strings_cache: dict) -> dict:
    """Extract all objective strings using cache."""
    objectives = {}
    marker = b'\x01\x06\x07str.qst'
    pos = 0
    while True:
        pos = payload.find(marker, pos)
        if pos == -1:
            break
        if pos >= 1:
            id1 = payload[pos - 1]
            text = strings_cache.get((id1, stid))
            if text:
                objectives[id1] = text
        pos += 1
    return objectives


def extract_journal_fast(stid: int, strings_cache: dict) -> list:
    """Extract journal entry texts using cache."""
    entries = []
    for id1 in range(200, 300):
        text = strings_cache.get((id1, stid))
        if text and len(text) > 20:
            entries.append(text)
    return entries


def decode_quest_fast(payload: bytes, fqn: str, guid: str, strings_cache: dict) -> Optional[Quest]:
    """Decode a quest using pre-loaded strings cache."""
    class_code, planet = extract_class_and_planet(fqn)
    if not class_code or class_code not in CLASS_NAMES:
        return None

    stid = extract_string_table_id(payload) or 0

    # Get name from cache
    name = strings_cache.get((88, stid)) or fqn.split('.')[-1]

    # Extract components
    objectives = extract_objectives_fast(payload, stid, strings_cache)
    steps = extract_quest_steps(payload)
    milestones, prerequisites = extract_milestones(payload)
    guid_refs = extract_guid_refs(payload)
    journal = extract_journal_fast(stid, strings_cache)

    return Quest(
        fqn=fqn,
        guid=guid,
        name=name,
        class_code=class_code,
        class_name=CLASS_NAMES[class_code],
        faction=FACTION[class_code],
        planet=planet,
        string_table_id=stid,
        objectives=objectives,
        journal_entries=journal,
        steps=[{'branch': s.branch, 'step': s.step, 'task': s.task, 'name': s.name} for s in steps],
        milestones=milestones,
        prerequisites=prerequisites,
        spawn_points=extract_fqn_refs(payload, b'spn.'),
        npcs=extract_fqn_refs(payload, b'npc.'),
        mission_phases=extract_fqn_refs(payload, b'mpn.'),
        codex_entries=extract_fqn_refs(payload, b'cdx.'),
        guid_refs=guid_refs,
    )


def main():
    db_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    conn = sqlite3.connect(db_path)

    # Pre-load ALL strings into memory for fast lookup
    print("Loading strings into memory...")
    strings_cache = {}
    for id1, id2, text in conn.execute("SELECT id1, id2, text FROM strings"):
        strings_cache[(id1, id2)] = text
    print(f"  Loaded {len(strings_cache)} strings")

    # Get ALL origin story quests
    rows = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE fqn LIKE 'qst.location.%.class.%'
        AND kind = 'Quest'
        AND json_extract(json, '$.payload_b64') IS NOT NULL
        ORDER BY fqn
    """).fetchall()

    print(f"Processing {len(rows)} origin story quests...")

    # Decode all quests
    quests_by_class = defaultdict(list)
    for i, (payload_b64, fqn, guid) in enumerate(rows):
        if (i + 1) % 50 == 0:
            print(f"  Processed {i + 1}/{len(rows)}...")
        payload = base64.b64decode(payload_b64)
        quest = decode_quest_fast(payload, fqn, guid, strings_cache)
        if quest:
            quests_by_class[quest.class_code].append(quest)

    # Summary by class
    print(f"\n{'='*80}")
    print("ORIGIN STORY QUESTS BY CLASS")
    print(f"{'='*80}")

    total_quests = 0
    total_objectives = 0
    total_npcs = 0

    for class_code in sorted(quests_by_class.keys()):
        quests = quests_by_class[class_code]
        class_name = CLASS_NAMES[class_code]
        faction = FACTION[class_code]

        obj_count = sum(len(q.objectives) for q in quests)
        npc_count = sum(len(q.npcs) for q in quests)

        print(f"\n{class_name} ({faction}): {len(quests)} quests, {obj_count} objectives, {npc_count} NPC refs")

        # Group by planet
        by_planet = defaultdict(list)
        for q in quests:
            by_planet[q.planet].append(q)

        for planet in sorted(by_planet.keys()):
            pquests = by_planet[planet]
            print(f"  {planet}: {len(pquests)} quests")
            for q in pquests[:3]:
                print(f"    - {q.name}")
            if len(pquests) > 3:
                print(f"    ... and {len(pquests) - 3} more")

        total_quests += len(quests)
        total_objectives += obj_count
        total_npcs += npc_count

    print(f"\n{'='*80}")
    print(f"TOTALS: {total_quests} quests, {total_objectives} objectives, {total_npcs} NPC references")
    print(f"{'='*80}")

    # Export to JSON
    output_path = Path('all_origins_quests.json')
    export_data = {
        'meta': {
            'total_quests': total_quests,
            'total_objectives': total_objectives,
            'classes': list(quests_by_class.keys()),
        },
        'quests': []
    }

    for class_code in sorted(quests_by_class.keys()):
        for quest in quests_by_class[class_code]:
            export_data['quests'].append({
                'fqn': quest.fqn,
                'guid': quest.guid,
                'name': quest.name,
                'class_code': quest.class_code,
                'class_name': quest.class_name,
                'faction': quest.faction,
                'planet': quest.planet,
                'string_table_id': quest.string_table_id,
                'objectives': quest.objectives,
                'journal_entries': quest.journal_entries,
                'steps': quest.steps,
                'milestones': quest.milestones,
                'prerequisites': quest.prerequisites,
                'spawn_points': quest.spawn_points,
                'npcs': quest.npcs,
                'mission_phases': quest.mission_phases,
                'codex_entries': quest.codex_entries,
                'guid_refs': quest.guid_refs,
            })

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\nExported to {output_path}")

    conn.close()


if __name__ == '__main__':
    main()
