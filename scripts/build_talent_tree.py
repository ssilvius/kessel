#!/usr/bin/env python3
"""Build talent tree structure from extracted data."""

import sqlite3
import json
from collections import defaultdict

DB_PATH = "data/spice.7.8b.v10.sqlite"

def humanize(slug):
    """Convert underscore_slug to Title Case."""
    return slug.replace("_", " ").title()

def main():
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    cur = conn.cursor()

    # Get all talents
    cur.execute("""
        SELECT fqn, string_id, icon_name, game_id
        FROM objects
        WHERE kind = 'Talent'
        ORDER BY fqn
    """)

    talents = cur.fetchall()
    print(f"Total talents: {len(talents)}")

    # Parse FQN structure
    # Format: tal.<class>.skill.<discipline_or_utility>.<talent_name>
    # Or: tal.spvp.<category>.<subcategory>.<name>

    classes = defaultdict(lambda: defaultdict(list))
    spvp = defaultdict(lambda: defaultdict(list))
    other = []

    for row in talents:
        fqn = row['fqn']
        parts = fqn.split(".")

        if len(parts) < 3:
            other.append(fqn)
            continue

        prefix = parts[1]  # class name or "spvp"

        if prefix == "spvp":
            # Starfighter talents: tal.spvp.<category>.<subcategory>.<name>
            if len(parts) >= 4:
                category = parts[2]
                subcategory = parts[3] if len(parts) > 3 else "general"
                name = parts[-1]
                spvp[category][subcategory].append({
                    "fqn": fqn,
                    "name": humanize(name),
                    "icon": row['icon_name'],
                    "game_id": row['game_id'],
                    "has_string": row['string_id'] is not None
                })
        else:
            # Class talents: tal.<class>.skill.<discipline>.<name>
            if len(parts) >= 4 and parts[2] == "skill":
                discipline = parts[3]
                name = parts[-1]
                classes[prefix][discipline].append({
                    "fqn": fqn,
                    "name": humanize(name),
                    "icon": row['icon_name'],
                    "game_id": row['game_id'],
                    "has_string": row['string_id'] is not None
                })
            else:
                other.append(fqn)

    # Print class talent structure
    print("\n=== CLASS TALENTS ===")
    for class_name in sorted(classes.keys()):
        print(f"\n{humanize(class_name)}:")
        disciplines = classes[class_name]
        for disc in sorted(disciplines.keys()):
            talents_list = disciplines[disc]
            with_string = sum(1 for t in talents_list if t['has_string'])
            with_icon = sum(1 for t in talents_list if t['icon'])
            print(f"  {humanize(disc)}: {len(talents_list)} talents ({with_string} w/string, {with_icon} w/icon)")
            # Show first 3
            for t in talents_list[:3]:
                marker = "*" if t['has_string'] else " "
                icon_marker = "I" if t['icon'] else " "
                print(f"    {marker}{icon_marker} {t['name']}")

    # Print SPVP talent structure
    print("\n\n=== STARFIGHTER TALENTS ===")
    for category in sorted(spvp.keys()):
        print(f"\n{humanize(category)}:")
        for subcat in sorted(spvp[category].keys()):
            talents_list = spvp[category][subcat]
            print(f"  {humanize(subcat)}: {len(talents_list)} talents")

    # Summary stats
    print("\n\n=== SUMMARY ===")
    total_class = sum(len(t) for d in classes.values() for t in d.values())
    total_spvp = sum(len(t) for d in spvp.values() for t in d.values())
    print(f"Class talents: {total_class}")
    print(f"Starfighter talents: {total_spvp}")
    print(f"Other/unstructured: {len(other)}")

    # Check utility vs discipline talents
    print("\n=== UTILITY vs DISCIPLINE ===")
    for class_name in sorted(classes.keys()):
        utility_count = 0
        discipline_count = 0
        for disc, talents_list in classes[class_name].items():
            if disc == "utility":
                utility_count = len(talents_list)
            else:
                discipline_count += len(talents_list)
        print(f"{humanize(class_name)}: {utility_count} utility, {discipline_count} discipline")

    # Show the "other" talents
    print("\n=== UNSTRUCTURED TALENTS ===")
    for fqn in sorted(other)[:20]:
        print(f"  {fqn}")
    if len(other) > 20:
        print(f"  ... and {len(other) - 20} more")

    conn.close()

if __name__ == "__main__":
    main()
