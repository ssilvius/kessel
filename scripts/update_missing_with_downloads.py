#!/usr/bin/env python3
"""
Update missing abilities JSON to include downloaded icon paths.
"""

import json
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
MISSING_ICONS_DIR = OUTPUT_DIR / "missing_icons"
PARSELY_ICONS_DIR = OUTPUT_DIR / "parsely_icons"

def main():
    # Load missing abilities
    missing_file = OUTPUT_DIR / "missing_abilities_from_parsely.json"
    with open(missing_file) as f:
        missing_abilities = json.load(f)

    # Build map of all downloaded icons (from both directories)
    # Use lowercase keys for case-insensitive matching
    downloaded = {}
    for icons_dir in [MISSING_ICONS_DIR, PARSELY_ICONS_DIR]:
        if icons_dir.exists():
            for f in icons_dir.glob('*.png'):
                url_lower = f"/img/icons/{f.name}".lower()
                downloaded[url_lower] = f.name

    print(f"Downloaded icons available: {len(downloaded)}")

    # Update abilities with downloaded icon info
    updated = 0
    for ability in missing_abilities:
        parsely_icon = ability.get('parsely_icon')
        # Add downloaded_icon for any ability that has a parsely_icon we downloaded
        # Use lowercase for case-insensitive matching
        if parsely_icon and parsely_icon.lower() in downloaded:
            ability['downloaded_icon'] = downloaded[parsely_icon.lower()]
            if not ability.get('local_icon_path'):
                updated += 1

    print(f"Updated abilities with downloaded icons: {updated}")

    # Stats
    with_local = sum(1 for a in missing_abilities if a.get('local_icon_path'))
    with_downloaded = sum(1 for a in missing_abilities if a.get('downloaded_icon'))
    no_icon = sum(1 for a in missing_abilities if not a.get('local_icon_path') and not a.get('downloaded_icon'))

    print(f"\nIcon coverage:")
    print(f"  With local match: {with_local}")
    print(f"  With downloaded: {with_downloaded}")
    print(f"  No icon: {no_icon}")
    print(f"  Total: {len(missing_abilities)}")

    # Save updated file
    output_file = OUTPUT_DIR / "missing_abilities_complete.json"
    with open(output_file, 'w') as f:
        json.dump(missing_abilities, f, indent=2)

    print(f"\nSaved to {output_file}")

    # Show abilities still without icons
    if no_icon > 0:
        print("\nAbilities still without icons:")
        for a in missing_abilities:
            if not a.get('local_icon_path') and not a.get('downloaded_icon'):
                print(f"  {a['name']} ({a['discipline']}): {a.get('parsely_icon', 'NO URL')}")

if __name__ == "__main__":
    main()
