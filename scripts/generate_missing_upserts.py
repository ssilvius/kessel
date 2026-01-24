#!/usr/bin/env python3
"""
Generate SQL upserts for missing discipline tree entries.

For each missing entry from Parsely:
1. If local_game_id exists, look up full record in spice.sqlite
2. If not, search spice.sqlite by name
3. If still not found, mark as synthetic (needs manual game_id)
4. Generate SQL for abilities/talents + discipline_tree_entries

Output: SQL migration file
"""

import hashlib
import json
import sqlite3
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR.parent / "output"
SPICE_DB = Path(__file__).parents[3] / "data" / "spice.sqlite"
D1_DB = Path(__file__).parents[3] / ".wrangler/state/v3/d1/miniflare-D1DatabaseObject/59bc915c66a73e797bab7ac753fba9590146218b72d1f74b7766de48f7d0cf26.sqlite"

# Discipline slug normalization (Parsely -> D1)
SLUG_MAP = {
    'innovative_ordnance': 'innovative-ordinance',
    'pyrotech': 'pyro-tech',
}

def get_discipline_id(cursor, parsely_slug: str) -> str | None:
    """Get discipline ID from D1 database."""
    # Convert Parsely slug (underscores) to D1 slug (hyphens)
    d1_slug = parsely_slug.replace('_', '-')
    # Apply known corrections
    if parsely_slug in SLUG_MAP:
        d1_slug = SLUG_MAP[parsely_slug]

    cursor.execute("SELECT id FROM disciplines WHERE slug = ?", (d1_slug,))
    row = cursor.fetchone()
    return row[0] if row else None

def escape_sql(s: str | None) -> str:
    """Escape string for SQL."""
    if s is None:
        return "NULL"
    # Escape single quotes and normalize whitespace (newlines break SQL)
    escaped = s.replace("'", "''")
    escaped = ' '.join(escaped.split())  # Normalize all whitespace to single spaces
    return "'" + escaped + "'"

def generate_synthetic_id(name: str, discipline: str) -> str:
    """Generate a synthetic game_id for entries not in extraction."""
    # Use sha256 of name:discipline, prefix with 'syn' marker
    h = hashlib.sha256(f"{name}:{discipline}".encode()).hexdigest()[:13]
    return f"syn{h}"

def main():
    # Load missing abilities
    missing_file = OUTPUT_DIR / "missing_abilities_from_parsely.json"
    if not missing_file.exists():
        print("Run find_missing_abilities.py first!")
        return

    with open(missing_file) as f:
        missing = json.load(f)

    print(f"Processing {len(missing)} missing entries")

    # Connect to databases
    spice = sqlite3.connect(SPICE_DB)
    spice_cur = spice.cursor()

    d1 = sqlite3.connect(D1_DB)
    d1_cur = d1.cursor()

    # Build spice lookups
    # By game_id
    spice_cur.execute("""
        SELECT game_id, fqn, guid, kind, icon_name
        FROM objects
        WHERE kind IN ('Ability', 'Talent')
    """)
    by_game_id = {}
    by_name = {}
    for game_id, fqn, guid, kind, icon_name in spice_cur.fetchall():
        by_game_id[game_id] = {
            'fqn': fqn,
            'guid': guid,
            'kind': kind,
            'icon_name': icon_name,
        }

    # By name (from strings table via string_id)
    spice_cur.execute("""
        SELECT s.text, o.game_id, o.fqn, o.guid, o.kind, o.icon_name
        FROM strings s
        JOIN objects o ON s.id2 = o.string_id
        WHERE o.kind IN ('Ability', 'Talent')
          AND s.locale = 'en-us'
          AND s.text != ''
    """)
    for name, game_id, fqn, guid, kind, icon_name in spice_cur.fetchall():
        name_lower = name.lower().strip()
        if name_lower not in by_name:
            by_name[name_lower] = []
        by_name[name_lower].append({
            'game_id': game_id,
            'fqn': fqn,
            'guid': guid,
            'kind': kind,
            'icon_name': icon_name,
        })

    print(f"Loaded {len(by_game_id)} objects by game_id")
    print(f"Loaded {len(by_name)} unique names")

    # Process each missing entry
    found_in_spice = []
    found_by_name = []
    synthetic = []

    for entry in missing:
        name = entry['name']
        discipline = entry['discipline']
        local_game_id = entry.get('local_game_id')

        # Get discipline ID
        disc_id = get_discipline_id(d1_cur, discipline)
        if not disc_id:
            print(f"  WARNING: No discipline found for {discipline}")
            continue

        entry['discipline_id'] = disc_id

        # Try lookup by local_game_id first
        if local_game_id and local_game_id in by_game_id:
            spice_data = by_game_id[local_game_id]
            entry['spice'] = spice_data
            entry['game_id'] = local_game_id
            entry['entry_type'] = 'talent' if spice_data['kind'] == 'Talent' else 'ability'
            found_in_spice.append(entry)
            continue

        # Try lookup by name
        name_lower = name.lower().strip()
        if name_lower in by_name:
            candidates = by_name[name_lower]
            # Prefer exact match, otherwise take first
            spice_data = candidates[0]
            entry['spice'] = spice_data
            entry['game_id'] = spice_data['game_id']
            entry['entry_type'] = 'talent' if spice_data['kind'] == 'Talent' else 'ability'
            found_by_name.append(entry)
            continue

        # Not found - synthetic
        entry['game_id'] = generate_synthetic_id(name, discipline)
        entry['entry_type'] = 'ability'  # Default to ability for synthetics
        entry['spice'] = None
        synthetic.append(entry)

    spice.close()
    d1.close()

    print(f"\nResults:")
    print(f"  Found by game_id: {len(found_in_spice)}")
    print(f"  Found by name: {len(found_by_name)}")
    print(f"  Synthetic (not in spice): {len(synthetic)}")

    # Generate SQL
    sql_lines = []
    sql_lines.append("-- Migration: Add missing discipline tree entries")
    sql_lines.append("-- Generated by generate_missing_upserts.py")
    sql_lines.append("")
    sql_lines.append("-- Section 1: Entries found in spice.sqlite (have real game_id)")
    sql_lines.append("")

    # Process found entries - just need discipline_tree_entries
    # The ability/talent should already exist in D1 from ETL
    all_found = found_in_spice + found_by_name

    for entry in all_found:
        disc_id = entry['discipline_id']
        game_id = entry['game_id']
        entry_type = entry['entry_type']
        unlock_level = entry.get('unlock_level') or 0
        tier_column = entry.get('tier_column') or 0
        is_passive = 1 if entry.get('category') == 'passive' else 0

        sql_lines.append(f"-- {entry['discipline']}:{entry['name']}")
        sql_lines.append(f"INSERT INTO discipline_tree_entries (discipline_id, entry_type, entry_id, unlock_level, tier_column, is_passive)")
        sql_lines.append(f"VALUES ('{disc_id}', '{entry_type}', '{game_id}', {unlock_level}, {tier_column}, {is_passive})")
        sql_lines.append(f"ON CONFLICT(discipline_id, entry_type, entry_id) DO UPDATE SET")
        sql_lines.append(f"  unlock_level = excluded.unlock_level,")
        sql_lines.append(f"  tier_column = excluded.tier_column,")
        sql_lines.append(f"  is_passive = excluded.is_passive;")
        sql_lines.append("")

    sql_lines.append("")
    sql_lines.append("-- Section 2: Synthetic entries (not in game extraction)")
    sql_lines.append("-- These need manual review - may be client-hardcoded")
    sql_lines.append("")

    for entry in synthetic:
        name = entry['name']
        discipline = entry['discipline']
        disc_id = entry['discipline_id']
        game_id = entry['game_id']
        description = entry.get('description', '')
        unlock_level = entry.get('unlock_level') or 0
        tier_column = entry.get('tier_column') or 0
        is_passive = 1 if entry.get('category') == 'passive' else 0
        # All synthetic entries now have icons (parsely downloads or manual fixes)
        icon_path = f"/icons/abilities/{game_id}.webp"

        # Create slug from name
        slug = name.lower().replace(' ', '-').replace("'", '')

        # Normalize description for comment (single line, truncated)
        desc_for_comment = ' '.join(description.split())[:80]
        sql_lines.append(f"-- SYNTHETIC: {discipline}:{name}")
        sql_lines.append(f"-- Description: {desc_for_comment}...")
        sql_lines.append(f"-- Icon: {icon_path or 'NONE'}")
        sql_lines.append(f"INSERT INTO abilities (id, fqn, source_guid, name, slug, description, icon_path, ability_type, is_passive, for_export)")
        sql_lines.append(f"VALUES (")
        sql_lines.append(f"  '{game_id}',")
        sql_lines.append(f"  'abl.synthetic.{discipline}.{slug}',")
        sql_lines.append(f"  'synthetic-{game_id}',")
        sql_lines.append(f"  {escape_sql(name)},")
        sql_lines.append(f"  '{slug}',")
        sql_lines.append(f"  {escape_sql(description)},")
        sql_lines.append(f"  {escape_sql(icon_path) if icon_path else 'NULL'},")
        sql_lines.append(f"  'discipline',")
        sql_lines.append(f"  {is_passive},")
        sql_lines.append(f"  1")
        sql_lines.append(f")")
        sql_lines.append(f"ON CONFLICT(id) DO UPDATE SET")
        sql_lines.append(f"  name = excluded.name,")
        sql_lines.append(f"  description = excluded.description,")
        sql_lines.append(f"  icon_path = COALESCE(excluded.icon_path, abilities.icon_path);")
        sql_lines.append("")
        sql_lines.append(f"INSERT INTO discipline_tree_entries (discipline_id, entry_type, entry_id, unlock_level, tier_column, is_passive)")
        sql_lines.append(f"VALUES ('{disc_id}', 'ability', '{game_id}', {unlock_level}, {tier_column}, {is_passive})")
        sql_lines.append(f"ON CONFLICT(discipline_id, entry_type, entry_id) DO UPDATE SET")
        sql_lines.append(f"  unlock_level = excluded.unlock_level,")
        sql_lines.append(f"  tier_column = excluded.tier_column,")
        sql_lines.append(f"  is_passive = excluded.is_passive;")
        sql_lines.append("")

    # Write SQL file
    sql_file = OUTPUT_DIR / "missing_tree_entries.sql"
    with open(sql_file, 'w') as f:
        f.write('\n'.join(sql_lines))

    print(f"\nGenerated SQL: {sql_file}")

    # Also save JSON for reference
    result = {
        'found_in_spice': found_in_spice,
        'found_by_name': found_by_name,
        'synthetic': synthetic,
    }
    json_file = OUTPUT_DIR / "missing_entries_processed.json"
    with open(json_file, 'w') as f:
        json.dump(result, f, indent=2)

    print(f"Saved JSON: {json_file}")

    # Show synthetic entries (need manual work)
    if synthetic:
        print(f"\nSynthetic entries needing review:")
        for s in synthetic[:20]:
            print(f"  {s['discipline']}:{s['name']} (L{s.get('unlock_level', '?')})")
        if len(synthetic) > 20:
            print(f"  ... and {len(synthetic) - 20} more")

if __name__ == "__main__":
    main()
