#!/usr/bin/env python3
"""
Match Parsely icons to local icons using perceptual hashing.
Requires: pip install imagehash pillow
"""

import json
from pathlib import Path

try:
    import imagehash
    from PIL import Image
except ImportError:
    print("Install dependencies: pip install imagehash pillow")
    exit(1)

# Paths
SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
PARSELY_ICONS_DIR = OUTPUT_DIR / "parsely_icons"
LOCAL_ICONS_DIR = Path("/Users/seansilvius/projects/ssilvius/huttspawn/public/icons")

def get_phash(image_path: Path) -> str | None:
    """Get perceptual hash of an image."""
    try:
        img = Image.open(image_path)
        return str(imagehash.phash(img))
    except Exception as e:
        print(f"  Error hashing {image_path}: {e}")
        return None

def main():
    # Load scraped abilities
    abilities_file = OUTPUT_DIR / "parsely_abilities_with_icons.json"
    if not abilities_file.exists():
        print("Run scrape_parsely_icons.py first!")
        return

    with open(abilities_file) as f:
        all_abilities = json.load(f)

    print("Building perceptual hash index of local icons...")

    # Index local icons by phash
    local_hash_map = {}  # phash -> list of (path, game_id)

    for icon_type in ['abilities', 'talents', 'misc']:
        icon_dir = LOCAL_ICONS_DIR / icon_type
        if not icon_dir.exists():
            continue

        files = list(icon_dir.glob('*.webp'))
        print(f"  Indexing {len(files)} {icon_type} icons...")

        for i, icon_file in enumerate(files):
            if i % 500 == 0 and i > 0:
                print(f"    {i}/{len(files)}...")

            phash = get_phash(icon_file)
            if phash:
                game_id = icon_file.stem  # filename without extension
                if phash not in local_hash_map:
                    local_hash_map[phash] = []
                local_hash_map[phash].append({
                    'path': f"/icons/{icon_type}/{icon_file.name}",
                    'game_id': game_id
                })

    print(f"Indexed {sum(len(v) for v in local_hash_map.values())} local icons with {len(local_hash_map)} unique hashes")

    # Match Parsely icons
    print("\nMatching Parsely icons to local icons...")

    # Get unique icon URLs and their files
    icon_matches = {}  # parsely_url -> {parsely_file, local_matches: [...]}

    parsely_files = list(PARSELY_ICONS_DIR.glob('*.png'))
    print(f"Processing {len(parsely_files)} Parsely icons...")

    for i, parsely_file in enumerate(parsely_files):
        if i % 100 == 0 and i > 0:
            print(f"  {i}/{len(parsely_files)}...")

        parsely_url = f"/img/icons/{parsely_file.name}"
        phash = get_phash(parsely_file)

        matches = []
        if phash and phash in local_hash_map:
            matches = local_hash_map[phash]

        icon_matches[parsely_url] = {
            'parsely_file': parsely_file.name,
            'phash': phash,
            'local_matches': matches
        }

    # Count matches
    matched = sum(1 for m in icon_matches.values() if m['local_matches'])
    print(f"\nMatched {matched}/{len(icon_matches)} Parsely icons to local icons")

    # Save full mapping
    mapping_file = OUTPUT_DIR / "parsely_to_local_icons.json"
    with open(mapping_file, 'w') as f:
        json.dump(icon_matches, f, indent=2)
    print(f"Saved mapping to {mapping_file}")

    # Create a simplified mapping for abilities
    print("\nCreating ability icon mapping...")

    ability_icon_map = {}  # ability_name -> {icon_path, game_id}

    for disc_name, abilities in all_abilities.items():
        for ability in abilities:
            name = ability.get('name', '')
            icon_url = ability.get('icon_url', '')
            desc = ability.get('description', '')

            if not name or not icon_url:
                continue

            match_data = icon_matches.get(icon_url, {})
            local_matches = match_data.get('local_matches', [])

            key = f"{disc_name}:{name}"
            ability_icon_map[key] = {
                'name': name,
                'discipline': disc_name,
                'description': desc,
                'parsely_icon': icon_url,
                'unlock_level': ability.get('unlock_level'),
                'tier_column': ability.get('tier_column'),
                'category': ability.get('category'),
                'local_icon': local_matches[0] if local_matches else None,
                'all_matches': local_matches
            }

    # Save ability mapping
    ability_map_file = OUTPUT_DIR / "ability_icon_mapping.json"
    with open(ability_map_file, 'w') as f:
        json.dump(ability_icon_map, f, indent=2)

    # Count abilities with matches
    with_icons = sum(1 for a in ability_icon_map.values() if a['local_icon'])
    with_desc = sum(1 for a in ability_icon_map.values() if a['description'])

    print(f"Saved ability mapping to {ability_map_file}")
    print(f"Total abilities: {len(ability_icon_map)}")
    print(f"With local icons: {with_icons}")
    print(f"With descriptions: {with_desc}")

    # Show some examples of matched abilities
    print("\nExample matches:")
    for key, data in list(ability_icon_map.items())[:5]:
        if data['local_icon']:
            print(f"  {data['name']}: {data['local_icon']['path']}")

    # Show unmatched abilities
    print("\nUnmatched abilities (first 10):")
    unmatched = [k for k, v in ability_icon_map.items() if not v['local_icon']]
    for key in unmatched[:10]:
        data = ability_icon_map[key]
        print(f"  {data['name']} ({data['discipline']}): {data['parsely_icon']}")

if __name__ == "__main__":
    main()
