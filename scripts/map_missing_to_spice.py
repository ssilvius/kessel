#!/usr/bin/env python3
"""
Map missing Parsely abilities to spice.sqlite data.

For each missing ability:
1. Search strings table by name to find string FQN
2. Find matching object (abl.* or tal.*)
3. Get game_id from object
4. Check if icon exists locally

Output: JSON with FQN, game_id, icon status for each ability.
"""

import json
import sqlite3
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
SPICE_DB = Path(__file__).parents[3] / "data" / "spice.sqlite"
LOCAL_ICONS_DIR = Path(__file__).parents[3] / "public" / "icons"

def find_icon_locally(game_id: str) -> str | None:
    """Check if icon exists in local folders."""
    for subdir in ['abilities', 'talents', 'misc']:
        path = LOCAL_ICONS_DIR / subdir / f"{game_id}.webp"
        if path.exists():
            return f"/icons/{subdir}/{game_id}.webp"
    return None

def main():
    # Load missing abilities
    missing_file = OUTPUT_DIR / "missing_abilities_complete.json"
    if not missing_file.exists():
        print("Run update_missing_with_downloads.py first!")
        return

    with open(missing_file) as f:
        missing_abilities = json.load(f)

    print(f"Loaded {len(missing_abilities)} missing abilities")

    # Connect to spice.sqlite
    conn = sqlite3.connect(SPICE_DB)
    cursor = conn.cursor()

    # Build name -> string FQN mapping for abilities and talents
    cursor.execute("""
        SELECT fqn, text FROM strings
        WHERE locale = 'en-us'
          AND (fqn LIKE 'str.abl.%' OR fqn LIKE 'str.tal.%')
    """)
    name_to_string_fqn = {}
    for fqn, text in cursor.fetchall():
        # Store by lowercase name for matching
        name_lower = text.lower().strip()
        if name_lower not in name_to_string_fqn:
            name_to_string_fqn[name_lower] = []
        name_to_string_fqn[name_lower].append(fqn)

    print(f"Loaded {len(name_to_string_fqn)} unique ability/talent names from strings")

    # Build object lookup by FQN
    cursor.execute("""
        SELECT fqn, game_id, icon_name, guid FROM objects
        WHERE kind IN ('abl', 'tal')
    """)
    objects = {}
    for fqn, game_id, icon_name, guid in cursor.fetchall():
        objects[fqn] = {
            'game_id': game_id,
            'icon_name': icon_name,
            'guid': guid
        }

    print(f"Loaded {len(objects)} ability/talent objects")

    conn.close()

    # Map missing abilities
    mapped = []
    not_found = []
    already_have_icon = []

    for ability in missing_abilities:
        name = ability['name']
        discipline = ability['discipline']
        name_lower = name.lower().strip()

        # Check if we already have a local icon match
        if ability.get('local_icon_path'):
            already_have_icon.append(ability)
            continue

        # Find string FQN candidates
        string_fqns = name_to_string_fqn.get(name_lower, [])

        # Try to find matching object
        matched_object = None
        matched_fqn = None

        for string_fqn in string_fqns:
            # Convert str.abl.X.Y.Z to abl.X.Y.Z or str.tal.X.Y.Z to tal.X.Y.Z
            obj_fqn = string_fqn.replace('str.', '', 1)

            # Direct match
            if obj_fqn in objects:
                matched_object = objects[obj_fqn]
                matched_fqn = obj_fqn
                break

            # Try variants (string FQN may have ID suffix like str.abl.0.123456)
            # Look for objects that match the pattern
            for obj_fqn_candidate, obj_data in objects.items():
                # Check if the object FQN ends with the ability name slug
                name_slug = name_lower.replace(' ', '_').replace("'", '')
                if obj_fqn_candidate.endswith(name_slug):
                    matched_object = obj_data
                    matched_fqn = obj_fqn_candidate
                    break

            if matched_object:
                break

        if matched_object:
            game_id = matched_object['game_id']
            local_icon = find_icon_locally(game_id)

            mapped.append({
                **ability,
                'spice_fqn': matched_fqn,
                'game_id': game_id,
                'icon_name': matched_object['icon_name'],
                'guid': matched_object['guid'],
                'local_icon_from_spice': local_icon,
            })
        else:
            not_found.append(ability)

    # Summary
    print(f"\nResults:")
    print(f"  Already have local icon: {len(already_have_icon)}")
    print(f"  Mapped to spice object: {len(mapped)}")
    print(f"  Not found in spice: {len(not_found)}")

    # Save results
    output_file = OUTPUT_DIR / "missing_abilities_spice_mapped.json"
    with open(output_file, 'w') as f:
        json.dump({
            'already_have_icon': already_have_icon,
            'mapped': mapped,
            'not_found': not_found
        }, f, indent=2)

    print(f"\nSaved to {output_file}")

    # Show sample of mapped
    print("\nSample mapped abilities:")
    for m in mapped[:10]:
        print(f"  {m['name']} ({m['discipline']})")
        print(f"    -> {m['spice_fqn']}")
        print(f"    -> game_id: {m['game_id']}")
        print(f"    -> local: {m.get('local_icon_from_spice', 'NONE')}")

    # Show sample of not found
    if not_found:
        print(f"\nNot found in spice (first 20):")
        for nf in not_found[:20]:
            print(f"  {nf['name']} ({nf['discipline']})")

if __name__ == "__main__":
    main()
