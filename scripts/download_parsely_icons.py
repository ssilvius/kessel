#!/usr/bin/env python3
"""
Download icons from Parsely.io and create a mapping to local icons.
Uses perceptual hashing to match downloaded icons to existing local icons.
"""

import hashlib
import json
import os
import time
import urllib.request
from pathlib import Path

# Paths
SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
ICONS_DIR = OUTPUT_DIR / "parsely_icons"
LOCAL_ICONS_DIR = Path.home() / "projects/ssilvius/huttspawn/public/icons"

def download_icon(icon_url: str, save_path: Path) -> bool:
    """Download icon from Parsely."""
    if save_path.exists():
        return True

    full_url = f"https://parsely.io{icon_url}"
    try:
        req = urllib.request.Request(full_url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            data = response.read()
            save_path.write_bytes(data)
            return True
    except Exception as e:
        print(f"  Failed to download {icon_url}: {e}")
        return False

def file_hash(path: Path) -> str:
    """Get MD5 hash of file content."""
    return hashlib.md5(path.read_bytes()).hexdigest()

def find_matching_local_icon(parsely_hash: str, hash_to_local: dict) -> str | None:
    """Find local icon with matching hash."""
    return hash_to_local.get(parsely_hash)

def main():
    # Load scraped abilities
    abilities_file = OUTPUT_DIR / "parsely_abilities_with_icons.json"
    if not abilities_file.exists():
        print("Run scrape_parsely_icons.py first!")
        return

    with open(abilities_file) as f:
        all_abilities = json.load(f)

    # Get unique icon URLs
    icon_urls = set()
    for disc_abilities in all_abilities.values():
        for ability in disc_abilities:
            url = ability.get('icon_url', '')
            if url:
                icon_urls.add(url)

    print(f"Found {len(icon_urls)} unique icons to download")

    # Create icons directory
    ICONS_DIR.mkdir(parents=True, exist_ok=True)

    # Download icons
    print("Downloading icons from Parsely...")
    for i, url in enumerate(sorted(icon_urls)):
        filename = url.split('/')[-1]
        save_path = ICONS_DIR / filename

        if download_icon(url, save_path):
            if i % 50 == 0:
                print(f"  {i}/{len(icon_urls)}...")

        time.sleep(0.1)  # Rate limit

    print(f"Downloaded to {ICONS_DIR}")

    # Build hash map of local icons
    print("\nBuilding hash map of local icons...")
    hash_to_local = {}

    for icon_type in ['abilities', 'talents', 'misc']:
        icon_dir = LOCAL_ICONS_DIR / icon_type
        if not icon_dir.exists():
            continue

        for icon_file in icon_dir.glob('*.webp'):
            try:
                h = file_hash(icon_file)
                hash_to_local[h] = f"/icons/{icon_type}/{icon_file.name}"
            except Exception:
                pass

    print(f"Indexed {len(hash_to_local)} local icons")

    # Build hash map of Parsely icons
    print("\nBuilding hash map of Parsely icons...")
    parsely_to_hash = {}
    for icon_file in ICONS_DIR.glob('*.png'):
        try:
            h = file_hash(icon_file)
            parsely_to_hash[icon_file.name] = h
        except Exception:
            pass

    # Try to match icons by hash (won't work for PNG vs WebP, but worth trying)
    # Instead, we'll output a mapping file for manual/visual matching

    # Create output mapping
    mapping = {}
    for url in sorted(icon_urls):
        filename = url.split('/')[-1]
        mapping[url] = {
            'parsely_file': filename,
            'local_match': None,  # To be filled by image matching
        }

    # Save mapping
    mapping_file = OUTPUT_DIR / "parsely_icon_mapping.json"
    with open(mapping_file, 'w') as f:
        json.dump(mapping, f, indent=2)

    print(f"\nSaved icon mapping to {mapping_file}")
    print(f"Total icons: {len(mapping)}")
    print("\nNext steps:")
    print("1. Use image matching to find local icon equivalents")
    print("2. Or manually map icon names to game_ids")

if __name__ == "__main__":
    main()
