#!/usr/bin/env python3
"""Decode ALL quest objects including mission phases (mpn.*)."""

import base64
import struct
import sqlite3
import sys
import json
import re
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass
from typing import Optional

def detect_quest_category(fqn: str) -> str:
    """Categorize quest by FQN prefix."""
    if fqn.startswith('qst.'):
        if '.class.' in fqn:
            return 'class_quest'
        if '.world_arc.' in fqn or '.world.' in fqn:
            return 'planetary_quest'
        if 'qst.exp.' in fqn:
            return 'expansion_quest'
        if 'qst.flashpoint.' in fqn:
            return 'flashpoint_quest'
        if 'qst.operation.' in fqn:
            return 'operation_quest'
        if 'qst.alliance.' in fqn:
            return 'alliance_quest'
        if 'qst.ventures.' in fqn:
            return 'venture_quest'
        if 'qst.daily_area.' in fqn:
            return 'daily_quest'
        if 'open_world' in fqn:
            return 'exploration_quest'
        return 'side_quest'
    elif fqn.startswith('mpn.'):
        if '.class.' in fqn:
            return 'class_phase'
        if '.world.' in fqn:
            return 'planetary_phase'
        if 'mpn.exp.' in fqn:
            return 'expansion_phase'
        if 'mpn.flashpoint.' in fqn:
            return 'flashpoint_phase'
        if 'mpn.daily_area.' in fqn:
            return 'daily_phase'
        if 'mpn.dynamic_events.' in fqn:
            return 'dynamic_event'
        return 'mission_phase'
    return 'unknown'


def detect_faction(fqn: str) -> Optional[str]:
    if '.imperial' in fqn or '.empire' in fqn:
        return 'Empire'
    if '.republic' in fqn:
        return 'Republic'
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
    # qst.location.<planet>.* or mpn.location.<planet>.*
    match = re.search(r'\.(location|daily_area)\.([^.]+)\.', fqn)
    if match:
        return match.group(2)
    # Expansion patterns
    match = re.search(r'\.exp\.\d+\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    return None


def extract_class_code(fqn: str) -> Optional[str]:
    match = re.search(r'\.class\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    return None


@dataclass
class QuestObject:
    fqn: str
    guid: str
    name: str
    category: str
    faction: Optional[str]
    planet: Optional[str]
    class_code: Optional[str]
    string_table_id: int
    objectives: dict
    steps: list
    milestones: list
    prerequisites: list
    spawn_points: list
    npcs: list
    mission_phases: list
    codex_entries: list
    guid_refs: list


def find_fqn_end(payload: bytes) -> Optional[int]:
    for prefix in [b'qst.', b'mpn.', b'npc.', b'abl.', b'itm.', b'spn.', b'cdx.']:
        pos = payload.find(prefix)
        if 0 < pos < 50:
            fqn_len = payload[pos - 1]
            if fqn_len > 0 and fqn_len < 200:
                return pos - 1 + fqn_len
    return None


def extract_string_table_id(payload: bytes) -> Optional[int]:
    fqn_end = find_fqn_end(payload)
    if fqn_end is None or fqn_end + 17 > len(payload):
        return None
    id2_offset = fqn_end + 12
    if id2_offset + 5 > len(payload):
        return None
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
    # Try both qst and mpn string markers
    for marker in [b'\x01\x06\x07str.qst', b'\x01\x06\x07str.mpn']:
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
                    steps.append({'branch': branch, 'step': step, 'task': task, 'name': step_name})
    return sorted(steps, key=lambda s: (s['branch'], s['step'], s['task']))


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


def decode_quest_object(payload: bytes, fqn: str, guid: str, strings_cache: dict) -> QuestObject:
    stid = extract_string_table_id(payload) or 0
    name = strings_cache.get((88, stid)) or fqn.split('.')[-1]

    objectives = extract_objectives_fast(payload, stid, strings_cache)
    steps = extract_quest_steps(payload)
    milestones, prerequisites = extract_milestones(payload)
    guid_refs = extract_guid_refs(payload)

    return QuestObject(
        fqn=fqn,
        guid=guid,
        name=name,
        category=detect_quest_category(fqn),
        faction=detect_faction(fqn),
        planet=extract_planet(fqn),
        class_code=extract_class_code(fqn),
        string_table_id=stid,
        objectives=objectives,
        steps=steps,
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

    # Get ALL quest objects (qst.* and mpn.*)
    rows = conn.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE kind = 'Quest'
        AND json_extract(json, '$.payload_b64') IS NOT NULL
        ORDER BY fqn
    """).fetchall()

    print(f"Processing {len(rows)} quest objects...")

    # Decode all
    by_category = defaultdict(list)
    for i, (payload_b64, fqn, guid) in enumerate(rows):
        if (i + 1) % 2000 == 0:
            print(f"  Processed {i + 1}/{len(rows)}...")
        payload = base64.b64decode(payload_b64)
        obj = decode_quest_object(payload, fqn, guid, strings_cache)
        by_category[obj.category].append(obj)

    # Summary
    print(f"\n{'='*80}")
    print("QUEST OBJECTS BY CATEGORY")
    print(f"{'='*80}")

    total = 0
    total_objectives = 0

    for category in sorted(by_category.keys()):
        objects = by_category[category]
        obj_count = sum(len(o.objectives) for o in objects)
        print(f"\n{category}: {len(objects)} objects, {obj_count} objectives")

        # Show planet breakdown for main categories
        if category.endswith('_quest') or category.endswith('_phase'):
            by_planet = defaultdict(int)
            for o in objects:
                by_planet[o.planet or 'unknown'] += 1
            for planet in sorted(by_planet.keys())[:5]:
                print(f"  {planet}: {by_planet[planet]}")
            if len(by_planet) > 5:
                print(f"  ... and {len(by_planet) - 5} more locations")

        total += len(objects)
        total_objectives += obj_count

    print(f"\n{'='*80}")
    print(f"TOTALS: {total} quest objects, {total_objectives} objectives")
    print(f"{'='*80}")

    # Export
    output_path = Path('all_quest_objects.json')
    export_data = {
        'meta': {
            'total': total,
            'total_objectives': total_objectives,
            'categories': {k: len(v) for k, v in by_category.items()},
        },
        'quests': [],
        'phases': []
    }

    for category in sorted(by_category.keys()):
        for obj in by_category[category]:
            record = {
                'fqn': obj.fqn,
                'guid': obj.guid,
                'name': obj.name,
                'category': obj.category,
                'faction': obj.faction,
                'planet': obj.planet,
                'class_code': obj.class_code,
                'string_table_id': obj.string_table_id,
                'objectives': obj.objectives,
                'steps': obj.steps,
                'milestones': obj.milestones,
                'prerequisites': obj.prerequisites,
                'spawn_points': obj.spawn_points,
                'npcs': obj.npcs,
                'mission_phases': obj.mission_phases,
                'codex_entries': obj.codex_entries,
                'guid_refs': obj.guid_refs,
            }
            if category.endswith('_phase') or category == 'dynamic_event':
                export_data['phases'].append(record)
            else:
                export_data['quests'].append(record)

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\nExported to {output_path}")
    print(f"  Quests: {len(export_data['quests'])}")
    print(f"  Phases: {len(export_data['phases'])}")

    conn.close()


if __name__ == '__main__':
    main()
