# Kessel Development Status

**Date**: 2025-12-12
**Status**: Complete - Ready for ETL

## Extraction Results

Full extraction completed with quality filters:

| Type | Count | Notes |
|------|-------|-------|
| Items | 94,011 | Gear, mods, tacticals, consumables |
| NPCs | 34,582 | Companions, quest NPCs, vendors |
| Schematics | 13,773 | Crafting recipes |
| Quests | 10,130 | Story, side, daily quests |
| Achievements | 6,107 | All achievement types |
| Codex | 3,152 | Lore entries |
| Abilities | 2,712 | Class, companion, legacy abilities |
| **Total Objects** | **164,492** | |
| Strings | 554,980 | Localized text (en-us) |

**Database**: 145MB SQLite file
**Runtime**: ~81 minutes for 101 .tor files

## Quality Filters Applied

### Object Type Filter
Only extract: `abl`, `itm`, `npc`, `schem`, `qst`, `cdx`, `ach`, `mpn`

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
- **Problem**: Brute-force frame size detection (50→3000 attempts per object)
- **Solution**: Expanding rings search from content size estimate
- **Result**: ~1 second per bucket vs 30+ minutes

## Next Steps

1. **TypeScript ETL** - Join objects to strings, output to D1 schema
2. **Parse GOM Payloads** - Extract structured data from binary format

## Commands

```bash
# Build
cd ~/projects/ssilvius/huttspawn/tools/kessel
cargo build --release

# Full extraction
./target/release/kessel \
  --input ~/swtor/assets \
  --output ~/swtor/data/spice.sqlite \
  --hashes ~/swtor/data/hashes_filename.txt

# Check results
sqlite3 ~/swtor/data/spice.sqlite "
  SELECT kind, COUNT(*) as cnt FROM objects
  GROUP BY kind ORDER BY cnt DESC;
"
```
