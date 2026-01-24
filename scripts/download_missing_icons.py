#!/usr/bin/env python3
"""
Download missing icons from Parsely for abilities that don't have local icon matches.
"""

import json
import time
import urllib.request
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
MISSING_ICONS_DIR = OUTPUT_DIR / "missing_icons"

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

def main():
    # Load missing abilities
    missing_file = OUTPUT_DIR / "missing_abilities_from_parsely.json"
    if not missing_file.exists():
        print("Run find_missing_abilities.py first!")
        return

    with open(missing_file) as f:
        missing_abilities = json.load(f)

    # Find abilities without local icon matches
    need_download = []
    for ability in missing_abilities:
        if not ability.get('local_icon_path') and ability.get('parsely_icon'):
            need_download.append(ability)

    print(f"Total missing abilities: {len(missing_abilities)}")
    print(f"Without local icon match: {len(need_download)}")

    # Get unique icon URLs
    icon_urls = set(a['parsely_icon'] for a in need_download)
    print(f"Unique icons to download: {len(icon_urls)}")

    # Create output directory
    MISSING_ICONS_DIR.mkdir(parents=True, exist_ok=True)

    # Download icons
    print("\nDownloading icons from Parsely...")
    downloaded = 0
    failed = 0

    for i, url in enumerate(sorted(icon_urls)):
        filename = url.split('/')[-1]
        save_path = MISSING_ICONS_DIR / filename

        if download_icon(url, save_path):
            downloaded += 1
            if (i + 1) % 10 == 0:
                print(f"  {i + 1}/{len(icon_urls)}...")
        else:
            failed += 1

        time.sleep(0.1)  # Rate limit

    print(f"\nDownloaded: {downloaded}")
    print(f"Failed: {failed}")
    print(f"Saved to: {MISSING_ICONS_DIR}")

    # List what was downloaded
    print("\nDownloaded icons:")
    for f in sorted(MISSING_ICONS_DIR.glob('*.png'))[:20]:
        print(f"  {f.name}")
    remaining = len(list(MISSING_ICONS_DIR.glob('*.png'))) - 20
    if remaining > 0:
        print(f"  ... and {remaining} more")

if __name__ == "__main__":
    main()
