#!/usr/bin/env python3
"""
Convert downloaded Parsely icons to game_id format for synthetic entries.

For each synthetic entry:
1. Find the downloaded PNG (from parsely_icon or local_icon_path)
2. Convert PNG -> WebP
3. Rename to game_id.webp
4. Copy to ~/swtor/data/icons/abilities/

Run import-icons.sh after to sync to R2.
"""

import json
import subprocess
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
ICONS_DEST = Path.home() / "swtor/data/icons/abilities"
PARSELY_ICONS_DIR = OUTPUT_DIR / "parsely_icons"
MISSING_ICONS_DIR = OUTPUT_DIR / "missing_icons"

def find_source_icon(entry: dict) -> Path | None:
    """Find source PNG for this entry."""
    # Try downloaded Parsely icon first
    parsely_icon = entry.get('parsely_icon', '')
    if parsely_icon:
        # /img/icons/foo.png -> foo.png
        filename = parsely_icon.split('/')[-1].lower()

        for icons_dir in [MISSING_ICONS_DIR, PARSELY_ICONS_DIR]:
            # Case-insensitive search
            if icons_dir.exists():
                for f in icons_dir.glob('*.png'):
                    if f.name.lower() == filename:
                        return f

    # Try local icon path (already exists as webp)
    local_path = entry.get('local_icon_path', '')
    if local_path:
        # /icons/abilities/abc123.webp -> check if exists
        full_path = Path(__file__).parents[3] / "public" / local_path.lstrip('/')
        if full_path.exists():
            return full_path

    return None

def convert_to_webp(src: Path, dest: Path) -> bool:
    """Convert image to WebP using cwebp or PIL."""
    try:
        # Try cwebp first (better quality)
        result = subprocess.run(
            ['cwebp', '-q', '90', str(src), '-o', str(dest)],
            capture_output=True
        )
        if result.returncode == 0:
            return True
    except FileNotFoundError:
        pass

    # Fallback to PIL
    try:
        from PIL import Image
        img = Image.open(src)
        img.save(dest, 'WEBP', quality=90)
        return True
    except Exception as e:
        print(f"  Error converting {src}: {e}")
        return False

def main():
    # Load processed entries
    processed_file = OUTPUT_DIR / "missing_entries_processed.json"
    if not processed_file.exists():
        print("Run generate_missing_upserts.py first!")
        return

    with open(processed_file) as f:
        data = json.load(f)

    synthetic = data.get('synthetic', [])
    print(f"Processing {len(synthetic)} synthetic entries")

    # Ensure destination exists
    ICONS_DEST.mkdir(parents=True, exist_ok=True)

    converted = 0
    skipped = 0
    no_source = 0
    already_exists = 0

    for entry in synthetic:
        game_id = entry['game_id']
        name = entry['name']
        dest_path = ICONS_DEST / f"{game_id}.webp"

        # Skip if already exists
        if dest_path.exists():
            already_exists += 1
            continue

        # Find source
        src = find_source_icon(entry)
        if not src:
            print(f"  No icon source for: {name}")
            no_source += 1
            continue

        # Convert/copy
        if src.suffix.lower() == '.webp':
            # Already webp, just copy
            import shutil
            shutil.copy(src, dest_path)
            converted += 1
        else:
            # Convert PNG to WebP
            if convert_to_webp(src, dest_path):
                converted += 1
            else:
                skipped += 1

    print(f"\nResults:")
    print(f"  Converted: {converted}")
    print(f"  Already existed: {already_exists}")
    print(f"  No source icon: {no_source}")
    print(f"  Skipped (error): {skipped}")
    print(f"\nIcons saved to: {ICONS_DEST}")
    print(f"Run: ./scripts/import-icons.sh to sync to R2")

if __name__ == "__main__":
    main()
