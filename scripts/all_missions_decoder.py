#!/usr/bin/env python3
"""Decode ALL missions from GOM data - class, planetary, expansion, flashpoint, etc."""

import base64
import struct
import sqlite3
import sys
import json
import re
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Optional

# Mission type detection from FQN
def detect_mission_type(fqn: str) -> str:
    """Detect mission type from FQN pattern."""
    if '.class.' in fqn:
        return 'class'
    if '.world_arc.' in fqn or '.world.' in fqn:
        return 'planetary'
    if 'qst.exp.' in fqn:
        return 'expansion'
    if 'qst.flashpoint.' in fqn:
        return 'flashpoint'
    if 'qst.operation.' in fqn:
        return 'operation'
    if 'qst.event.' in fqn:
        return 'event'
    if 'qst.alliance.' in fqn:
        return 'alliance'
    if 'qst.ventures.' in fqn:
        return 'venture'
    if 'open_world' in fqn:
        return 'exploration'
    return 'side'


def detect_faction(fqn: str) -> Optional[str]:
    """Detect faction from FQN."""
    if '.imperial' in fqn or '.empire' in fqn:
        return 'Empire'
    if '.republic' in fqn:
        return 'Republic'
    # Class-based detection
    empire_classes = ['sith_warrior', 'sith_sorcerer', 'bounty_hunter', 'spy']
    republic_classes = ['jedi_knight', 'jedi_wizard', 'jedi_consular', 'smuggler', 'trooper']
    for c in empire_classes:
        if f'.{c}.' in fqn:
            return 'Empire'
    for c in republic_classes:
        if f'.{c}.' in fqn:
            return 'Republic'
    return None


def extract_planet(fqn: str) -> Optional[str]:
    """Extract planet from FQN."""
    # Pattern: qst.location.<planet>.*
    match = re.search(r'qst\.location\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    # Expansion patterns
    match = re.search(r'qst\.exp\.\d+\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    return None


def extract_class_code(fqn: str) -> Optional[str]:
    """Extract class code from FQN."""
    match = re.search(r'\.class\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    return None


def extract_expansion(fqn: str) -> Optional[str]:
    """Extract expansion identifier from FQN."""
    # qst.exp.01.makeb, qst.exp.02.rishi, etc.
    match = re.search(r'qst\.exp\.(\d+)\.([^.]+)', fqn)
    if match:
        return f"exp{match.group(1)}_{match.group(2)}"
    return None


@dataclass
class QuestStep:
    branch: int
    step: int
    task: int
    name: str


@dataclass
class Mission:
    """Complete decoded mission."""
    fqn: str
    guid: str
    name: str
    mission_type: str
    faction: Optional[str]
    planet: Optional[str]
    class_code: Optional[str]
    expansion: Optional[str]
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
    guid_refs: list


def find_fqn_end(payload: bytes) -> Optional[int]:
    for prefix in [b'qst.', b'npc.', b'abl.', b'itm.', b'mpn.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if 0 < pos < 30:
            fqn_len = payload[pos - 1]
            return pos - 1 + fqn_len
    return None


def extract_string_table_id(payload: bytes) -> Optional[int]:
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
        return int.from_bytes(payload[id2_offset+1:id2_offset+4], 'big')
    return None


def extract_objectives_fast(payload: bytes, stid: int, strings_cache: dict) -> dict:
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


def extract_quest_steps(payload: bytes) -> list:
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


def extract_journal_fast(stid: int, strings_cache: dict) -> list:
    entries = []
    for id1 in range(200, 300):
        text = strings_cache.get((id1, stid))
        if text and len(text) > 20:
            entries.append(text)
    return entries


def decode_mission(payload: bytes, fqn: str, guid: str, strings_cache: dict) -> Mission:
    """Decode a complete mission structure."""
    stid = extract_string_table_id(payload) or 0
    name = strings_cache.get((88, stid)) or fqn.split('.')[-1]

    objectives = extract_objectives_fast(payload, stid, strings_cache)
    steps = extract_quest_steps(payload)
    milestones, prerequisites = extract_milestones(payload)
    guid_refs = extract_guid_refs(payload)
    journal = extract_journal_fast(stid, strings_cache)

    return Mission(
        fqn=fqn,
        guid=guid,
        name=name,
        mission_type=detect_mission_type(fqn),
        faction=detect_faction(fqn),
        planet=extract_planet(fqn),
        class_code=extract_class_code(fqn),
        expansion=extract_expansion(fqn),
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

    # Pre-load strings
    print("Loading strings into memory...")
    strings_cache = {}
    for id1, id2, text in conn.execute("SELECT id1, id2, text FROM strings"):
        strings_cache[(id1, id2)] = text
    print(f"  Loaded {len(strings_cache)} strings")

    # Get ALL quests with payloads
    rows = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE fqn LIKE 'qst.%'
        AND kind = 'Quest'
        AND json_extract(json, '$.payload_b64') IS NOT NULL
        ORDER BY fqn
    """).fetchall()

    print(f"Processing {len(rows)} missions...")

    # Decode all missions
    missions_by_type = defaultdict(list)
    for i, (payload_b64, fqn, guid) in enumerate(rows):
        if (i + 1) % 200 == 0:
            print(f"  Processed {i + 1}/{len(rows)}...")
        payload = base64.b64decode(payload_b64)
        mission = decode_mission(payload, fqn, guid, strings_cache)
        missions_by_type[mission.mission_type].append(mission)

    # Summary by type
    print(f"\n{'='*80}")
    print("MISSIONS BY TYPE")
    print(f"{'='*80}")

    total_missions = 0
    total_objectives = 0

    for mission_type in sorted(missions_by_type.keys()):
        missions = missions_by_type[mission_type]
        obj_count = sum(len(m.objectives) for m in missions)

        print(f"\n{mission_type.upper()}: {len(missions)} missions, {obj_count} objectives")

        # Group by planet
        by_planet = defaultdict(list)
        for m in missions:
            planet = m.planet or 'unknown'
            by_planet[planet].append(m)

        for planet in sorted(by_planet.keys())[:10]:
            print(f"  {planet}: {len(by_planet[planet])} missions")
        if len(by_planet) > 10:
            print(f"  ... and {len(by_planet) - 10} more planets")

        total_missions += len(missions)
        total_objectives += obj_count

    print(f"\n{'='*80}")
    print(f"TOTALS: {total_missions} missions, {total_objectives} objectives")
    print(f"{'='*80}")

    # Export to JSON
    output_path = Path('all_missions.json')
    export_data = {
        'meta': {
            'total_missions': total_missions,
            'total_objectives': total_objectives,
            'types': {k: len(v) for k, v in missions_by_type.items()},
        },
        'missions': []
    }

    for mission_type in sorted(missions_by_type.keys()):
        for mission in missions_by_type[mission_type]:
            export_data['missions'].append({
                'fqn': mission.fqn,
                'guid': mission.guid,
                'name': mission.name,
                'mission_type': mission.mission_type,
                'faction': mission.faction,
                'planet': mission.planet,
                'class_code': mission.class_code,
                'expansion': mission.expansion,
                'string_table_id': mission.string_table_id,
                'objectives': mission.objectives,
                'journal_entries': mission.journal_entries,
                'steps': mission.steps,
                'milestones': mission.milestones,
                'prerequisites': mission.prerequisites,
                'spawn_points': mission.spawn_points,
                'npcs': mission.npcs,
                'mission_phases': mission.mission_phases,
                'codex_entries': mission.codex_entries,
                'guid_refs': mission.guid_refs,
            })

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\nExported to {output_path}")

    conn.close()


if __name__ == '__main__':
    main()
