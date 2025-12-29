# Kessel Development Status

**Date**: 2025-12-28
**Status**: Complete - Production Ready

## Extraction Results

Full extraction completed with quality filters (SWTOR 7.8b):

| Type | Count | Notes |
|------|-------|-------|
| Items | 98,692 | Gear, mods, tacticals, consumables |
| NPCs | 36,242 | Companions, quest NPCs, vendors |
| Quests | 11,692 | Story, side, daily quests |
| Abilities | 2,893 | Class, companion, legacy abilities |
| **Total Objects** | **174,824** | |
| Strings | 557,325 | Localized text (en-us) |
| Icons | 900+ | WebP format, case-insensitive matching |

**Database**: ~350MB SQLite file
**Runtime**: ~80 minutes for 101 .tor files

## Recent Changes

### Icon Extraction (2025-12-28)
- Case-insensitive icon name matching (fixes mixed-case FQNs like `abl_jk_ZealousLeap`)
- Icons saved by `game_id` (16-char hex hash)
- Organized by object kind (`abilities/`, `items/`, `talents/`, etc.)
- Shared icons saved for ALL referencing objects

### Grammar Processing (2025-12-26)
- Embedded at compile time (`include_str!`)
- No CLI flag needed - always active
- Cleans SWTOR template syntax from descriptions
- Rules in `grammar.toml`

## Quality Filters Applied

### Object Type Filter
Only extract: `abl`, `itm`, `npc`, `schem`, `qst`, `cdx`, `ach`, `mpn`, `tal`

### Version Deduplication
Skip FQNs containing `/` (versioned duplicates like `abl.foo.bar/17/5`)

### Test/Debug Content
Skip objects with: `test`, `debug`, `deprecated`, `obsolete`, `qa`, `dev` in FQN

### Ability Filters (34 internal prefixes)
- NPC/encounter: `npc`, `qtr`, `operation`, `flashpoint`, `creature`
- Internal systems: `command`, `conquest`, `mtx`, `pvp`, `event`, `player`
- Quest/area mechanics: `exp`, `quest`, `daily_area`, `alliance`
- And more: `e3`, `galactic_seasons`, `gld`, `itm`, `reputation`, `stronghold`, etc.

### Item Filters (10 internal prefixes)
- Condition checks: `has_item`, `slot_is_lowest`, `slot_is_rating`, `irating`
- Internal: `npc`, `loot`, `ach`, `codex`, `mercury`, `location`

### NPC Filters (5 internal prefixes)
- Templates: `blueprints`, `ability`, `combat`, `cinematic_extras`, `heavy_weight_cos`

## Performance Improvements

### ZSTD Parsing Fix
- **Problem**: Brute-force frame size detection (50->3000 attempts per object)
- **Solution**: Expanding rings search from content size estimate
- **Result**: ~1 second per bucket vs 30+ minutes

## Commands

```bash
# Build
cd ~/projects/ssilvius/huttspawn/tools/kessel
cargo build --release

# Full extraction with icons
./target/release/kessel \
  --input ~/swtor/assets \
  --output ~/swtor/data/spice.sqlite \
  --hashes ~/swtor/data/hashes_filename.txt \
  --icons \
  --icons-output ~/swtor/data/icons

# Check results
sqlite3 ~/swtor/data/spice.sqlite "
  SELECT kind, COUNT(*) as cnt FROM objects
  GROUP BY kind ORDER BY cnt DESC;
"

# Copy icons to web app
cp -r ~/swtor/data/icons/* ~/projects/ssilvius/huttspawn/public/icons/
```

## Next Steps

1. **Re-run extraction** - With case-insensitive icon matching to get all ~900 icons
2. **ETL to D1** - TypeScript scripts to transform and load into Cloudflare D1
3. **MDX Integration** - Strong.astro component resolves ability names to icons/tooltips
