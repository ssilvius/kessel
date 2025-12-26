#!/usr/bin/env python3
"""
Build discipline talent trees from extracted SWTOR data.
Decodes GOM payloads to extract talent levels and organize into trees.
"""

import sqlite3
import json
import base64
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Optional


# Discipline mapping: FQN component -> (Empire name, Republic mirror)
DISCIPLINE_MIRRORS = {
    # Sith Inquisitor / Jedi Consular
    "corruption": ("Corruption", "Seer"),
    "seer": ("Seer", "Corruption"),
    "lightning": ("Lightning", "Telekinetics"),
    "telekinetics": ("Telekinetics", "Lightning"),
    "madness": ("Madness", "Balance"),
    "balance": ("Balance", "Madness"),

    # Sith Warrior / Jedi Knight
    "immortal": ("Immortal", "Defense"),
    "defense": ("Defense", "Immortal"),
    "vengeance": ("Vengeance", "Vigilance"),
    "vigilance": ("Vigilance", "Vengeance"),
    "rage": ("Rage", "Focus"),
    "focus": ("Focus", "Rage"),

    # Bounty Hunter / Trooper
    "bodyguard": ("Bodyguard", "Combat Medic"),
    "combat_medic": ("Combat Medic", "Bodyguard"),
    "arsenal": ("Arsenal", "Gunnery"),
    "gunnery": ("Gunnery", "Arsenal"),
    "innovative_ordnance": ("Innovative Ordnance", "Assault Specialist"),
    "assault_specialist": ("Assault Specialist", "Innovative Ordnance"),

    # Powertech / Vanguard
    "shield_tech": ("Shield Tech", "Shield Specialist"),
    "shield_specialist": ("Shield Specialist", "Shield Tech"),
    "advanced_prototype": ("Advanced Prototype", "Tactics"),
    "tactics": ("Tactics", "Advanced Prototype"),
    "pyrotech": ("Pyrotech", "Plasmatech"),
    "plasmatech": ("Plasmatech", "Pyrotech"),

    # Operative / Scoundrel
    "medicine": ("Medicine", "Sawbones"),
    "sawbones": ("Sawbones", "Medicine"),
    "concealment": ("Concealment", "Scrapper"),
    "scrapper": ("Scrapper", "Concealment"),
    "lethality": ("Lethality", "Ruffian"),
    "ruffian": ("Ruffian", "Lethality"),

    # Sniper / Gunslinger
    "engineering": ("Engineering", "Saboteur"),
    "saboteur": ("Saboteur", "Engineering"),
    "marksmanship": ("Marksmanship", "Sharpshooter"),
    "sharpshooter": ("Sharpshooter", "Marksmanship"),
    "virulence": ("Virulence", "Dirty Fighting"),
    "dirty_fighting": ("Dirty Fighting", "Virulence"),

    # Assassin / Shadow
    "darkness": ("Darkness", "Kinetic Combat"),
    "kinetic_combat": ("Kinetic Combat", "Darkness"),
    "deception": ("Deception", "Infiltration"),
    "infiltration": ("Infiltration", "Deception"),
    "hatred": ("Hatred", "Serenity"),
    "serenity": ("Serenity", "Hatred"),

    # Marauder / Sentinel
    "annihilation": ("Annihilation", "Watchman"),
    "watchman": ("Watchman", "Annihilation"),
    "carnage": ("Carnage", "Combat"),
    "combat": ("Combat", "Carnage"),
    "fury": ("Fury", "Concentration"),
    "concentration": ("Concentration", "Fury"),
}

# Discipline levels (based on game's discipline tree UI)
# Core talents are auto-granted at specific levels
DISCIPLINE_LEVELS = [15, 23, 27, 35, 39, 43, 47, 51, 60, 64, 68, 73, 78]

# Class mapping
CLASS_MAP = {
    "sith_warrior": ("Sith Warrior", "Empire"),
    "jedi_knight": ("Jedi Knight", "Republic"),
    "sith_inquisitor": ("Sith Inquisitor", "Empire"),
    "jedi_consular": ("Jedi Consular", "Republic"),
    "bounty_hunter": ("Bounty Hunter", "Empire"),
    "trooper": ("Trooper", "Republic"),
    "agent": ("Imperial Agent", "Empire"),
    "smuggler": ("Smuggler", "Republic"),
}


@dataclass
class Talent:
    fqn: str
    name: str
    discipline: str
    class_name: str
    level: Optional[int] = None
    abilities: list = field(default_factory=list)
    strings: list = field(default_factory=list)

    def display_name(self) -> str:
        """Convert FQN name to display name."""
        name = self.fqn.split('.')[-1]
        return name.replace('_', ' ').title()


def extract_level_from_payload(payload_b64: str) -> Optional[int]:
    """Extract talent level from base64-encoded GOM payload.

    Pattern from MAPPINGS.md:
    cf 40 00 00 40 d9 54 fb 02 05 [LEVEL]
    ^                             ^
    |_ Type ID D954FB02           |_ Level byte
    """
    try:
        payload = base64.b64decode(payload_b64)
    except Exception:
        return None

    # Exact pattern from MAPPINGS.md: cf 40 00 00 40 d9 54 fb 02 05
    pattern = bytes.fromhex('cf40000040d954fb0205')

    idx = payload.find(pattern)
    if idx != -1 and idx + len(pattern) < len(payload):
        level = payload[idx + len(pattern)]
        if level in DISCIPLINE_LEVELS:
            return level

    return None


def extract_ability_refs(payload_b64: str) -> list:
    """Extract ability GUID references from payload.

    Pattern from MAPPINGS.md:
    d0 01 cf e0 00 [8-byte GUID big-endian]
    The E000 is part of the marker, GUID bytes follow.
    Full GUID = E000 + 8 bytes (16 hex chars total)
    """
    try:
        payload = base64.b64decode(payload_b64)
    except Exception:
        return []

    abilities = []
    # Pattern: d0 01 cf e0 00 - then 6 more GUID bytes (E000 + 6 = 8 total)
    pattern = bytes([0xd0, 0x01, 0xcf, 0xe0, 0x00])

    idx = 0
    while True:
        idx = payload.find(pattern, idx)
        if idx == -1:
            break

        # The cf marker starts a 9-byte GUID: cf + 8 bytes
        # E0 00 are first 2 bytes, then 6 more follow
        guid_start = idx + 3  # Skip d0 01 cf, start at e0 00
        if guid_start + 8 <= len(payload):
            guid_bytes = payload[guid_start:guid_start + 8]
            # Big-endian: E0 00 XX XX XX XX XX XX -> E000XXXXXXXXXXXX
            guid = ''.join(f'{b:02X}' for b in guid_bytes)
            abilities.append(guid)

        idx += 1

    return abilities


def parse_discipline_from_fqn(fqn: str) -> tuple[Optional[str], Optional[str]]:
    """Parse discipline and class from talent FQN.

    Returns: (discipline_name, class_name) or (None, None)
    """
    parts = fqn.split('.')
    if len(parts) < 4 or parts[0] != 'tal':
        return None, None

    class_name = parts[1]

    # Pattern: tal.CLASS.skill.DISCIPLINE.talent_name
    if len(parts) >= 5 and parts[2] == 'skill':
        discipline = parts[3]
        if discipline in DISCIPLINE_MIRRORS:
            return discipline, class_name

    return None, class_name


def build_discipline_trees(db_path: str) -> dict:
    """Build discipline talent trees from database."""
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()

    # Get all talent objects
    cursor.execute("""
        SELECT fqn, json FROM objects
        WHERE kind = 'tal'
        ORDER BY fqn
    """)

    talents_by_discipline = defaultdict(list)

    for fqn, json_str in cursor.fetchall():
        data = json.loads(json_str)

        discipline, class_name = parse_discipline_from_fqn(fqn)
        if not discipline:
            continue

        payload_b64 = data.get('payload_b64', '')
        level = extract_level_from_payload(payload_b64)
        abilities = extract_ability_refs(payload_b64)
        strings = data.get('strings', [])

        talent = Talent(
            fqn=fqn,
            name=fqn.split('.')[-1],
            discipline=discipline,
            class_name=class_name,
            level=level,
            abilities=abilities,
            strings=strings
        )

        key = f"{class_name}.{discipline}"
        talents_by_discipline[key].append(talent)

    conn.close()
    return talents_by_discipline


def lookup_ability_names(db_path: str, guids: set) -> dict:
    """Look up ability names by GUID."""
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()

    guid_to_name = {}
    for guid in guids:
        cursor.execute("""
            SELECT fqn FROM objects
            WHERE guid = ? OR guid LIKE ?
        """, (guid, f"%{guid}%"))

        row = cursor.fetchone()
        if row:
            name = row[0].split('.')[-1]
            guid_to_name[guid] = name

    conn.close()
    return guid_to_name


def print_discipline_tree(discipline_key: str, talents: list, ability_names: dict):
    """Print a formatted discipline tree."""
    class_name, discipline = discipline_key.split('.', 1)

    disc_info = DISCIPLINE_MIRRORS.get(discipline, (discipline.title(), "Unknown"))
    class_info = CLASS_MAP.get(class_name, (class_name.replace('_', ' ').title(), "Unknown"))

    print(f"\n{'='*60}")
    print(f"  {disc_info[0]} ({class_info[0]} - {class_info[1]})")
    print(f"  Mirror: {disc_info[1]}")
    print(f"{'='*60}")

    # Group by level
    by_level = defaultdict(list)
    for talent in talents:
        by_level[talent.level].append(talent)

    # Print in level order
    for level in DISCIPLINE_LEVELS:
        if level in by_level:
            print(f"\n  Level {level}:")
            for talent in sorted(by_level[level], key=lambda t: t.name):
                print(f"    - {talent.display_name()}")
                if talent.abilities:
                    for guid in talent.abilities[:3]:  # Show first 3
                        abl_name = ability_names.get(guid, guid[:16])
                        print(f"        Grants/Modifies: {abl_name}")

    # Unknown level talents
    if None in by_level:
        print(f"\n  Core/Auto-granted:")
        for talent in sorted(by_level[None], key=lambda t: t.name):
            print(f"    - {talent.display_name()}")


def extract_all_effect_levels(payload_b64: str) -> list:
    """Extract all effect levels from payload."""
    try:
        payload = base64.b64decode(payload_b64)
    except Exception:
        return []

    pattern = bytes.fromhex('cf40000040d954fb0205')
    levels = []

    pos = 0
    while True:
        pos = payload.find(pattern, pos)
        if pos == -1:
            break
        level_pos = pos + len(pattern)
        if level_pos < len(payload):
            levels.append(payload[level_pos])
        pos += 1

    return levels


def build_comprehensive_trees(db_path: str) -> dict:
    """Build comprehensive talent data with all effect levels and ability refs."""
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()

    # Get all talent objects
    cursor.execute("""
        SELECT fqn, guid, json FROM objects
        WHERE kind = 'tal'
        ORDER BY fqn
    """)

    talents_by_discipline = defaultdict(list)
    ability_guid_cache = {}

    for fqn, guid, json_str in cursor.fetchall():
        data = json.loads(json_str)

        discipline, class_name = parse_discipline_from_fqn(fqn)
        if not discipline:
            continue

        payload_b64 = data.get('payload_b64', '')
        effect_levels = extract_all_effect_levels(payload_b64)
        abilities = extract_ability_refs(payload_b64)
        strings = data.get('strings', [])

        # Look up ability names
        ability_info = []
        for abl_guid in abilities:
            if abl_guid not in ability_guid_cache:
                row = cursor.execute(
                    "SELECT fqn FROM objects WHERE guid = ?", (abl_guid,)
                ).fetchone()
                ability_guid_cache[abl_guid] = row[0] if row else None
            ability_info.append({
                "guid": abl_guid,
                "fqn": ability_guid_cache[abl_guid]
            })

        talent_data = {
            "fqn": fqn,
            "guid": guid,
            "name": fqn.split('.')[-1].replace('_', ' ').title(),
            "effect_levels": effect_levels,
            "abilities": ability_info,
            "strings": strings
        }

        key = f"{class_name}.{discipline}"
        talents_by_discipline[key].append(talent_data)

    conn.close()
    return talents_by_discipline


def main():
    db_path = "data/spice.7.8b.v8.sqlite"

    print("Building comprehensive discipline talent data...")
    trees = build_comprehensive_trees(db_path)

    print(f"\nFound {len(trees)} disciplines")

    # Statistics
    total_talents = sum(len(t) for t in trees.values())
    with_effects = sum(1 for talents in trees.values() for t in talents if t['effect_levels'])
    with_abilities = sum(1 for talents in trees.values() for t in talents if t['abilities'])

    print(f"Total discipline talents: {total_talents}")
    print(f"Talents with effect levels: {with_effects}")
    print(f"Talents with ability refs: {with_abilities}")

    # Effect level distribution
    all_levels = []
    for talents in trees.values():
        for t in talents:
            all_levels.extend(t['effect_levels'])

    if all_levels:
        from collections import Counter
        level_counts = Counter(all_levels)
        print(f"\nTop 10 effect levels:")
        for level, count in level_counts.most_common(10):
            print(f"  {level}: {count}")

    # Build output structure
    output = {"disciplines": {}, "meta": {}}

    for key, talents in trees.items():
        class_name, discipline = key.split('.', 1)
        disc_info = DISCIPLINE_MIRRORS.get(discipline, (discipline.title(), None))
        class_info = CLASS_MAP.get(class_name, (class_name.replace('_', ' ').title(), "Unknown"))

        output["disciplines"][key] = {
            "name": disc_info[0],
            "mirror": disc_info[1],
            "class": class_info[0],
            "faction": class_info[1],
            "talents": sorted(talents, key=lambda x: x['name'])
        }

    output["meta"] = {
        "total_disciplines": len(trees),
        "total_talents": total_talents,
        "discipline_tiers": list(DISCIPLINE_LEVELS),
        "notes": [
            "effect_levels are scaling thresholds from payload, not tree positions",
            "Tree tier positions may need external mapping (Jedipedia/manual)",
            "abilities list contains GUID and FQN for granted/modified abilities"
        ]
    }

    # Export
    with open("tools/kessel/output/discipline_trees.json", "w") as f:
        json.dump(output, f, indent=2)

    print(f"\nExported to tools/kessel/output/discipline_trees.json")

    # Print sample
    print("\n" + "="*60)
    print("SAMPLE: Corruption (Sith Inquisitor)")
    print("="*60)

    if "sith_inquisitor.corruption" in trees:
        for talent in sorted(trees["sith_inquisitor.corruption"], key=lambda x: x['name']):
            print(f"\n{talent['name']}")
            print(f"  FQN: {talent['fqn']}")
            if talent['effect_levels']:
                print(f"  Effect levels: {talent['effect_levels']}")
            if talent['abilities']:
                for abl in talent['abilities']:
                    print(f"  Modifies: {abl['fqn'] or abl['guid']}")


if __name__ == "__main__":
    main()
