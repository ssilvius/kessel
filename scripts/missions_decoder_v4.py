#!/usr/bin/env python3
"""
Comprehensive mission decoder v4.

Combines:
1. Extracted quest objects (qst.*)
2. Constructed heroic quests from mission phases (mpn.*)
3. Alliance companion recruitment quests

This creates a complete mission dataset for seeding the D1 database.
"""

import base64
import struct
import sqlite3
import json
import re
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, asdict
from typing import Optional

@dataclass
class Mission:
    """Complete mission data."""
    fqn: str
    guid: str
    name: str
    mission_type: str
    faction: Optional[str]
    planet: Optional[str]
    class_code: Optional[str]
    string_table_id: int
    source: str  # 'extracted', 'constructed', 'companion'

def detect_mission_type(fqn: str, name: str = "") -> str:
    """Detect mission type from FQN pattern and name."""
    fqn_lower = fqn.lower()
    name_lower = name.lower() if name else ""

    # Check name prefixes first
    if name_lower.startswith('[heroic') or name_lower.startswith('[area'):
        return 'heroic'
    if name_lower.startswith('[daily'):
        return 'daily'
    if name_lower.startswith('[weekly'):
        return 'weekly'

    # FQN-based detection
    if '.class.' in fqn_lower:
        return 'class'
    if '.world_arc.' in fqn_lower or ('.world.' in fqn_lower and 'location' in fqn_lower):
        return 'planetary'
    if 'qst.exp.' in fqn_lower:
        return 'expansion'
    if 'qst.flashpoint.' in fqn_lower:
        return 'flashpoint'
    if 'qst.operation.' in fqn_lower:
        return 'operation'
    if 'qst.event.' in fqn_lower:
        return 'event'
    if 'qst.alliance.' in fqn_lower:
        if 'companion' in fqn_lower:
            return 'companion'
        return 'alliance'
    if 'qst.ventures.' in fqn_lower:
        return 'venture'
    if 'qst.daily_area.' in fqn_lower:
        return 'daily'
    if 'qst.heroic.' in fqn_lower:
        return 'heroic'
    if 'qst.qtr.' in fqn_lower:
        return 'weekly'
    return 'side'

def detect_faction(fqn: str) -> Optional[str]:
    """Detect faction from FQN."""
    fqn_lower = fqn.lower()
    if '.imperial' in fqn_lower or '.empire' in fqn_lower or '_imp' in fqn_lower:
        return 'empire'
    if '.republic' in fqn_lower or '_rep' in fqn_lower:
        return 'republic'

    empire_classes = ['sith_warrior', 'sith_sorcerer', 'sith_inquisitor', 'bounty_hunter', 'spy', 'agent']
    republic_classes = ['jedi_knight', 'jedi_wizard', 'jedi_consular', 'smuggler', 'trooper']

    for c in empire_classes:
        if f'.{c}.' in fqn_lower:
            return 'empire'
    for c in republic_classes:
        if f'.{c}.' in fqn_lower:
            return 'republic'
    return None

def extract_planet(fqn: str) -> Optional[str]:
    """Extract planet from FQN."""
    patterns = [
        r'qst\.location\.([^.]+)\.',
        r'qst\.daily_area\.([^.]+)\.',
        r'qst\.exp\.\d+\.([^.]+)\.',
    ]
    for pattern in patterns:
        match = re.search(pattern, fqn)
        if match:
            return match.group(1)
    return None

def extract_class_code(fqn: str) -> Optional[str]:
    """Extract class code from FQN."""
    match = re.search(r'\.class\.([^.]+)\.', fqn)
    if match:
        return match.group(1)
    return None

def is_internal_quest(fqn: str) -> bool:
    """Filter out truly internal/system quests."""
    fqn_lower = fqn.lower()
    internal_patterns = [
        'debug_', 'test_', '.hiddenquest', '.companion_unlock',
        '.unlock_', '_unlock_', 'training_', '.cutscene.', '.globals',
    ]
    return any(p in fqn_lower for p in internal_patterns)

def extract_string_table_id(payload: bytes, fqn: str) -> Optional[int]:
    """Find string table ID from payload."""
    fqn_bytes = fqn.encode('ascii')
    fqn_pos = payload.find(fqn_bytes)
    if fqn_pos < 0:
        return None

    fqn_end = fqn_pos + len(fqn_bytes)
    for offset in range(fqn_end, min(fqn_end + 40, len(payload) - 4)):
        if payload[offset] == 0xCE:
            stid = int.from_bytes(payload[offset+1:offset+4], 'big')
            if 1000 < stid < 10000000:
                return stid
    return None

def main():
    db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
    db = sqlite3.connect(db_path)

    # Load strings cache
    print("Loading strings...")
    strings_cache = {}
    for id1, id2, text in db.execute("SELECT id1, id2, text FROM strings"):
        strings_cache[(id1, id2)] = text
    print(f"  Loaded {len(strings_cache)} strings")

    missions = []
    stats = defaultdict(int)

    # Part 1: Extract quest objects
    print("\nExtracting quest objects (qst.*)...")
    rows = db.execute("""
        SELECT json_extract(json, '$.payload_b64'), fqn, guid
        FROM objects
        WHERE fqn LIKE 'qst.%'
        AND kind = 'Quest'
        AND json_extract(json, '$.payload_b64') IS NOT NULL
        ORDER BY fqn
    """).fetchall()

    for payload_b64, fqn, guid in rows:
        if is_internal_quest(fqn):
            stats['skipped_internal'] += 1
            continue

        payload = base64.b64decode(payload_b64)
        stid = extract_string_table_id(payload, fqn) or 0
        name = strings_cache.get((88, stid))

        if not name:
            stats['skipped_no_name'] += 1
            continue

        mission = Mission(
            fqn=fqn,
            guid=guid,
            name=name,
            mission_type=detect_mission_type(fqn, name),
            faction=detect_faction(fqn),
            planet=extract_planet(fqn),
            class_code=extract_class_code(fqn),
            string_table_id=stid,
            source='extracted'
        )
        missions.append(mission)
        stats['extracted'] += 1

    print(f"  Extracted: {stats['extracted']}")
    print(f"  Skipped (internal): {stats['skipped_internal']}")
    print(f"  Skipped (no name): {stats['skipped_no_name']}")

    # Part 2: Construct heroic quests from mission phases
    print("\nConstructing heroic quests from mission phases...")

    # Get heroic strings
    heroic_strings = {}
    for stid, name in db.execute(
        "SELECT id2, text FROM strings WHERE id1 = 88 AND text LIKE '[HEROIC%'"
    ):
        normalized = name.lower().replace('[heroic 2+] ', '').replace('[heroic 4] ', '')
        normalized = normalized.replace(' ', '_').replace("'", "").replace('-', '_')
        heroic_strings[normalized] = (stid, name)

    # Get mission phases
    mpn_by_quest = defaultdict(list)
    for (fqn,) in db.execute(
        "SELECT fqn FROM objects WHERE fqn LIKE 'mpn.location.%.world.%' OR fqn LIKE 'mpn.location.%.bronze.%'"
    ):
        parts = fqn.split('.')
        if len(parts) >= 5:
            planet = parts[2]
            quest_name = parts[4]
            mpn_by_quest[(planet, quest_name)].append(fqn)

    # Match heroics to mission phases
    extracted_fqns = {m.fqn for m in missions}

    for (planet, quest_name), phases in mpn_by_quest.items():
        # Skip if we already have this quest
        constructed_fqn = f"qst.location.{planet}.world.{quest_name}"
        if constructed_fqn in extracted_fqns:
            continue

        # Find matching heroic string
        quest_words = set(quest_name.split('_'))
        display_name = None
        best_stid = 0

        for hero_norm, (stid, hero_name) in heroic_strings.items():
            hero_words = set(hero_norm.split('_'))
            common_words = quest_words & hero_words
            if not common_words:
                continue

            score = len(common_words) / max(len(quest_words), len(hero_words))
            if quest_name in hero_norm or hero_norm in quest_name:
                score += 0.5

            if score >= 0.5 and (display_name is None or score > 0.5):
                display_name = hero_name
                best_stid = stid

        if not display_name:
            continue

        # Detect faction
        faction = None
        first_phase = phases[0] if phases else ""
        if '.imperial.' in first_phase or '.empire.' in first_phase:
            faction = 'empire'
        elif '.republic.' in first_phase:
            faction = 'republic'

        mission = Mission(
            fqn=constructed_fqn,
            guid='',  # No GUID for constructed quests
            name=display_name,
            mission_type='heroic',
            faction=faction,
            planet=planet,
            class_code=None,
            string_table_id=best_stid,
            source='constructed'
        )
        missions.append(mission)
        stats['constructed'] += 1

    print(f"  Constructed heroics: {stats['constructed']}")

    # Summary
    print(f"\n{'='*60}")
    print("SUMMARY")
    print(f"{'='*60}")

    by_type = defaultdict(list)
    for m in missions:
        by_type[m.mission_type].append(m)

    for mtype in sorted(by_type.keys()):
        count = len(by_type[mtype])
        print(f"  {mtype}: {count}")

    print(f"\n  TOTAL: {len(missions)} missions")

    # Export
    output_path = Path(__file__).parent / 'all_missions_v4.json'
    export_data = {
        'meta': {
            'total': len(missions),
            'extracted': stats['extracted'],
            'constructed': stats['constructed'],
            'skipped_internal': stats['skipped_internal'],
            'skipped_no_name': stats['skipped_no_name'],
            'by_type': {k: len(v) for k, v in by_type.items()},
        },
        'missions': [asdict(m) for m in missions]
    }

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\nExported to {output_path}")

    db.close()

if __name__ == '__main__':
    main()
