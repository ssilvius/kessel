#!/usr/bin/env python3
"""
Find abilities in Parsely that don't exist in our database.
Outputs JSON with all missing abilities including matched icons and descriptions.

Expected: 48 disciplines × 38 abilities each = 1,824 total entries
"""

import json
import sqlite3
from pathlib import Path

# Paths
SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
DB_PATH = Path(__file__).parents[3] / ".wrangler/state/v3/d1/miniflare-D1DatabaseObject/59bc915c66a73e797bab7ac753fba9590146218b72d1f74b7766de48f7d0cf26.sqlite"

def main():
    # Load Parsely abilities with improved icon matching
    abilities_file = OUTPUT_DIR / "ability_icon_mapping_improved.json"
    if not abilities_file.exists():
        print("Run match_parsely_icons.py first!")
        return

    with open(abilities_file) as f:
        parsely_abilities = json.load(f)

    # Get all abilities AND talents from database via discipline_tree_entries
    conn = sqlite3.connect(DB_PATH)
    cursor = conn.cursor()

    # Normalize discipline slug to match Parsely format
    # DB slugs use hyphens, Parsely uses underscores
    # DB also has name typos that propagate to slug: "innovative-ordinance", "pyro-tech"
    def normalize_slug(slug: str) -> str:
        s = slug.replace('-', '_')
        # Fix slugs derived from incorrect names
        s = s.replace('innovative_ordinance', 'innovative_ordnance')  # Ordinance -> Ordnance
        s = s.replace('pyro_tech', 'pyrotech')  # Pyro Tech -> Pyrotech
        return s

    # Query abilities - use slug instead of fqn
    cursor.execute("""
        SELECT d.slug as discipline_slug, a.name, a.id
        FROM discipline_tree_entries dte
        JOIN disciplines d ON dte.discipline_id = d.id
        JOIN abilities a ON dte.entry_id = a.id
        WHERE dte.entry_type = 'ability'
    """)

    db_entries = set()
    for row in cursor.fetchall():
        disc_slug, name, entry_id = row
        disc_normalized = normalize_slug(disc_slug)
        db_entries.add(f"{disc_normalized}:{name}")

    ability_count = len(db_entries)

    # Query talents - use slug instead of fqn
    cursor.execute("""
        SELECT d.slug as discipline_slug, t.name, t.id
        FROM discipline_tree_entries dte
        JOIN disciplines d ON dte.discipline_id = d.id
        JOIN talents t ON dte.entry_id = t.id
        WHERE dte.entry_type = 'talent'
    """)

    for row in cursor.fetchall():
        disc_slug, name, entry_id = row
        disc_normalized = normalize_slug(disc_slug)
        db_entries.add(f"{disc_normalized}:{name}")

    talent_count = len(db_entries) - ability_count

    conn.close()

    print(f"Database has {ability_count} abilities + {talent_count} talents = {len(db_entries)} total entries")
    print(f"Parsely has {len(parsely_abilities)} abilities")

    # Find what Parsely has that we don't
    missing_from_db = []
    for key, data in parsely_abilities.items():
        disc, name = key.split(':', 1)
        normalized_key = f"{disc}:{name}"

        if normalized_key not in db_entries:
            missing_from_db.append({
                'key': key,
                'name': data['name'],
                'discipline': data['discipline'],
                'description': data['description'],
                'local_icon_path': data.get('local_icon_path'),
                'local_game_id': data.get('local_game_id'),
                'parsely_icon': data.get('parsely_icon'),
                'unlock_level': data.get('unlock_level'),
                'tier_column': data.get('tier_column'),
                'category': data.get('category'),
            })

    print(f"\nMissing from database: {len(missing_from_db)} abilities")

    # Stats
    with_icons = sum(1 for a in missing_from_db if a['local_icon_path'])
    with_desc = sum(1 for a in missing_from_db if a['description'])
    print(f"With matched icons: {with_icons}")
    print(f"With descriptions: {with_desc}")

    # Group by discipline
    by_disc = {}
    for m in missing_from_db:
        disc = m['discipline']
        if disc not in by_disc:
            by_disc[disc] = []
        by_disc[disc].append(m)

    print("\nBy discipline:")
    for disc in sorted(by_disc.keys()):
        abilities = by_disc[disc]
        print(f"  {disc}: {len(abilities)}")
        for a in abilities[:3]:
            icon_status = "ICON" if a['local_icon_path'] else "NO ICON"
            print(f"    - {a['name']} (L{a['unlock_level']}, {a['category']}) [{icon_status}]")
        if len(abilities) > 3:
            print(f"    ... and {len(abilities) - 3} more")

    # Save missing abilities
    output_file = OUTPUT_DIR / "missing_abilities_from_parsely.json"
    with open(output_file, 'w') as f:
        json.dump(missing_from_db, f, indent=2)

    print(f"\nSaved to {output_file}")

if __name__ == "__main__":
    main()
