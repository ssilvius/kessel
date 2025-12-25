#!/usr/bin/env python3
"""
Construct heroic quest data from mission phases, strings, and NPCs.

This is a workaround because kessel doesn't extract prototype nodes where
the actual heroic quest objects are stored. We can reconstruct most of the
data from what we DO have.
"""

import sqlite3
import json
import re
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, asdict
from typing import Optional

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'

@dataclass
class HeroicQuest:
    """A reconstructed heroic quest."""
    fqn: str  # Constructed from mpn pattern
    name: str  # From [HEROIC 2+] strings
    planet: str
    faction: Optional[str]
    quest_giver: Optional[str]
    mission_phases: list  # mpn.* FQNs
    npcs: list  # npc.* FQNs

def normalize_name(name: str) -> str:
    """Normalize quest name for matching."""
    return (name.lower()
            .replace('[heroic 2+] ', '')
            .replace('[heroic 4] ', '')
            .replace(' ', '_')
            .replace("'", "")
            .replace('-', '_')
            .replace(':', ''))

def main():
    db = sqlite3.connect(db_path)

    # Step 1: Get all heroic strings
    print("Loading heroic strings...")
    heroic_strings = {}
    for stid, name in db.execute(
        "SELECT id2, text FROM strings WHERE id1 = 88 AND text LIKE '[HEROIC%'"
    ):
        normalized = normalize_name(name)
        heroic_strings[normalized] = (stid, name)

    print(f"  Found {len(heroic_strings)} heroic names")

    # Step 2: Get all world mission phases
    print("\nAnalyzing mission phases...")
    mpn_by_quest = defaultdict(list)
    quest_planets = {}

    for (fqn,) in db.execute(
        "SELECT fqn FROM objects WHERE fqn LIKE 'mpn.location.%.world.%' OR fqn LIKE 'mpn.location.%.bronze.%'"
    ):
        # Parse: mpn.location.<planet>.world.<quest>.<phase>
        # or: mpn.location.<planet>.bronze.<quest>.<phase>
        parts = fqn.split('.')
        if len(parts) >= 5:
            planet = parts[2]
            quest_type = parts[3]  # 'world' or 'bronze'
            quest_name = parts[4]

            key = (planet, quest_name)
            mpn_by_quest[key].append(fqn)
            quest_planets[key] = planet

    print(f"  Found {len(mpn_by_quest)} unique quest/planet combinations")

    # Step 3: Get NPCs for each heroic
    print("\nLinking NPCs...")
    npc_by_quest = defaultdict(list)

    for (fqn,) in db.execute(
        "SELECT fqn FROM objects WHERE fqn LIKE 'npc.location.%.world.%' OR fqn LIKE 'npc.location.%.bronze.%'"
    ):
        parts = fqn.split('.')
        if len(parts) >= 5:
            planet = parts[2]
            quest_name = parts[4]
            key = (planet, quest_name)
            npc_by_quest[key].append(fqn)

    # Step 4: Construct heroic quests
    print("\nConstructing heroic quests...")
    heroic_quests = []

    for (planet, quest_name), phases in mpn_by_quest.items():
        # Try to find matching heroic string
        quest_words = set(quest_name.split('_'))
        display_name = None
        best_match_score = 0

        for hero_norm, (stid, hero_name) in heroic_strings.items():
            hero_words = set(hero_norm.split('_'))

            # Calculate word overlap score
            common_words = quest_words & hero_words
            if not common_words:
                continue

            score = len(common_words) / max(len(quest_words), len(hero_words))

            # Direct substring match bonus
            if quest_name in hero_norm or hero_norm in quest_name:
                score += 0.5

            if score > best_match_score and score >= 0.5:
                best_match_score = score
                display_name = hero_name

        if not display_name:
            # Skip non-heroic world quests
            continue

        # Detect faction from FQN
        faction = None
        first_phase = phases[0] if phases else ""
        if '.imperial.' in first_phase or '.empire.' in first_phase:
            faction = 'empire'
        elif '.republic.' in first_phase:
            faction = 'republic'

        # Construct FQN
        fqn = f"qst.location.{planet}.world.{quest_name}"

        # Get NPCs
        npcs = npc_by_quest.get((planet, quest_name), [])

        # Get quest giver (if we have NPC data)
        quest_giver = None
        for npc in npcs:
            # Look for typical quest giver patterns
            if any(x in npc for x in ['giver', 'contact', 'terminal', 'board']):
                quest_giver = npc
                break

        quest = HeroicQuest(
            fqn=fqn,
            name=display_name,
            planet=planet,
            faction=faction,
            quest_giver=quest_giver,
            mission_phases=phases,
            npcs=npcs,
        )
        heroic_quests.append(quest)

    print(f"\n{'='*60}")
    print(f"RESULTS")
    print(f"{'='*60}")
    print(f"Heroic quests constructed: {len(heroic_quests)}")

    # Group by planet
    by_planet = defaultdict(list)
    for q in heroic_quests:
        by_planet[q.planet].append(q)

    print(f"\nBy planet:")
    for planet in sorted(by_planet.keys()):
        quests = by_planet[planet]
        print(f"  {planet}: {len(quests)} heroics")
        for q in quests[:3]:
            print(f"    - {q.name}")
        if len(quests) > 3:
            print(f"    ... and {len(quests)-3} more")

    # Export to JSON
    output_path = Path(__file__).parent / 'heroic_quests_constructed.json'
    export_data = {
        'meta': {
            'source': 'Constructed from mpn/string/npc data',
            'total': len(heroic_quests),
            'by_planet': {p: len(qs) for p, qs in by_planet.items()},
        },
        'quests': [asdict(q) for q in heroic_quests]
    }

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\nExported to {output_path}")

    db.close()

if __name__ == '__main__':
    main()
