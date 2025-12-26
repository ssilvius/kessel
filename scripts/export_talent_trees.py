#!/usr/bin/env python3
"""Export talent trees to JSON for frontend consumption."""

import sqlite3
import json
from collections import defaultdict
from pathlib import Path

DB_PATH = "data/spice.7.8b.v10.sqlite"
OUTPUT_DIR = Path("src/data/talents")

# SWTOR class mappings
CLASS_NAMES = {
    "agent": {"name": "Imperial Agent", "faction": "empire", "mirror": "smuggler"},
    "bounty_hunter": {"name": "Bounty Hunter", "faction": "empire", "mirror": "trooper"},
    "sith_inquisitor": {"name": "Sith Inquisitor", "faction": "empire", "mirror": "jedi_consular"},
    "sith_warrior": {"name": "Sith Warrior", "faction": "empire", "mirror": "jedi_knight"},
    "smuggler": {"name": "Smuggler", "faction": "republic", "mirror": "agent"},
    "trooper": {"name": "Trooper", "faction": "republic", "mirror": "bounty_hunter"},
    "jedi_consular": {"name": "Jedi Consular", "faction": "republic", "mirror": "sith_inquisitor"},
    "jedi_knight": {"name": "Jedi Knight", "faction": "republic", "mirror": "sith_warrior"},
}

def humanize(slug):
    """Convert underscore_slug to Title Case."""
    return slug.replace("_", " ").title()

def get_string_name(cur, string_id):
    """Look up localized name from string_id."""
    if not string_id:
        return None
    cur.execute("SELECT text FROM strings WHERE id2 = ? AND fqn LIKE 'str.tal.0.%'", (string_id,))
    row = cur.fetchone()
    return row['text'] if row else None

def main():
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    cur = conn.cursor()

    # Ensure output directory exists
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # Get all talents
    cur.execute("""
        SELECT fqn, string_id, icon_name, game_id, guid
        FROM objects
        WHERE kind = 'Talent'
        ORDER BY fqn
    """)

    talents = cur.fetchall()
    print(f"Processing {len(talents)} talents...")

    # Build class talent trees
    trees = {}

    for row in talents:
        fqn = row['fqn']
        parts = fqn.split(".")

        if len(parts) < 4:
            continue  # Skip short FQNs

        class_slug = parts[1]

        # Skip non-class talents (spvp, gld, etc.)
        if class_slug not in CLASS_NAMES:
            continue

        # Must have .skill. structure
        if len(parts) < 5 or parts[2] != "skill":
            continue

        discipline_slug = parts[3]
        talent_slug = parts[-1]

        # Get or create class tree
        if class_slug not in trees:
            class_info = CLASS_NAMES[class_slug]
            trees[class_slug] = {
                "id": class_slug,
                "name": class_info["name"],
                "faction": class_info["faction"],
                "mirror": class_info["mirror"],
                "disciplines": {},
                "utility": []
            }

        # Get localized name or humanize from slug
        localized_name = get_string_name(cur, row['string_id'])
        name = localized_name or humanize(talent_slug)

        talent_data = {
            "id": row['game_id'],
            "fqn": fqn,
            "slug": talent_slug,
            "name": name,
            "icon": row['icon_name'],
            "hasLocalizedName": localized_name is not None
        }

        # Utility or discipline?
        if discipline_slug == "utility":
            trees[class_slug]["utility"].append(talent_data)
        else:
            if discipline_slug not in trees[class_slug]["disciplines"]:
                trees[class_slug]["disciplines"][discipline_slug] = {
                    "id": discipline_slug,
                    "name": humanize(discipline_slug),
                    "talents": []
                }
            trees[class_slug]["disciplines"][discipline_slug]["talents"].append(talent_data)

    # Write individual class files
    for class_slug, tree in trees.items():
        output_path = OUTPUT_DIR / f"{class_slug}.json"
        with open(output_path, 'w') as f:
            json.dump(tree, f, indent=2)

        disc_count = sum(len(d["talents"]) for d in tree["disciplines"].values())
        util_count = len(tree["utility"])
        print(f"  {tree['name']}: {disc_count} discipline, {util_count} utility talents")

    # Export guild perks
    guild_perks = {"categories": defaultdict(list)}

    cur.execute("""
        SELECT fqn, string_id, icon_name, game_id
        FROM objects
        WHERE kind = 'Talent' AND fqn LIKE 'tal.gld.perk.%'
        ORDER BY fqn
    """)

    for row in cur.fetchall():
        fqn = row['fqn']
        parts = fqn.split(".")
        if len(parts) >= 4:
            # tal.gld.perk.<category>_<level>
            perk_name = parts[3]
            # Split category from level (e.g., "conquest_point_increase_1" -> "conquest_point_increase", "1")
            parts_underscore = perk_name.rsplit("_", 1)
            if len(parts_underscore) == 2 and parts_underscore[1].isdigit():
                category = parts_underscore[0]
                level = int(parts_underscore[1])
            else:
                category = perk_name
                level = 0

            localized_name = get_string_name(cur, row['string_id'])
            name = localized_name or humanize(perk_name)

            guild_perks["categories"][category].append({
                "id": row['game_id'],
                "fqn": fqn,
                "name": name,
                "level": level,
                "icon": row['icon_name']
            })

    # Sort perks by level within each category
    for category in guild_perks["categories"]:
        guild_perks["categories"][category].sort(key=lambda x: x["level"])

    guild_perks["categories"] = dict(guild_perks["categories"])

    guild_path = OUTPUT_DIR / "guild_perks.json"
    with open(guild_path, 'w') as f:
        json.dump(guild_perks, f, indent=2)

    perk_count = sum(len(perks) for perks in guild_perks["categories"].values())
    print(f"  Guild Perks: {perk_count} perks in {len(guild_perks['categories'])} categories")

    # Write index file
    index = {
        "classes": {
            slug: {
                "name": tree["name"],
                "faction": tree["faction"],
                "mirror": tree["mirror"],
                "disciplineCount": len(tree["disciplines"]),
                "utilityCount": len(tree["utility"]),
                "totalTalents": sum(len(d["talents"]) for d in tree["disciplines"].values()) + len(tree["utility"])
            }
            for slug, tree in trees.items()
        },
        "factions": {
            "empire": [s for s, t in trees.items() if t["faction"] == "empire"],
            "republic": [s for s, t in trees.items() if t["faction"] == "republic"]
        },
        "guildPerks": {
            "categoryCount": len(guild_perks["categories"]),
            "totalPerks": perk_count
        }
    }

    index_path = OUTPUT_DIR / "index.json"
    with open(index_path, 'w') as f:
        json.dump(index, f, indent=2)

    print(f"\nExported {len(trees)} class talent trees + guild perks to {OUTPUT_DIR}/")

    conn.close()

if __name__ == "__main__":
    main()
