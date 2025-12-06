# Kessel

SWTOR data miner - extracts game objects from .tor archives to SQLite.

Named after the spice mines of Kessel, continuing the Star Wars mining theme from [bespin](https://github.com/kbatten/bespin).

## Project Context

### The Mission Tracker

Huttspawn needs a mission tracker for SWTOR players with 20+ characters scattered across different progress points. The goal:

1. **Visual GSAP timeline** showing Eras → Chapters → Missions hierarchy
2. **"What's next?" recommendations** based on prerequisites and completed missions
3. **Bulk operations** - mark entire eras complete, drill down to find exact stopping point
4. **Static SVG** built at Astro SSR time, runtime fetches only user progress overlay

Reddit validated demand - hundreds of users want this feature.

### Why Build a Dataminer?

We considered two approaches:

**Option A: CSV Import** (from community Google Sheets)
- Faster initial implementation
- BUT: Uses UUIDs, name-based dependency linking (fragile)
- Missing: abilities, items, icons
- Would require **complete rewrite** when switching to game IDs

**Option B: Dataminer** (extract from game files)
- More upfront work
- Gets proper game IDs (GUID, FQN) from day 1
- Complete dataset: missions, abilities, items, NPCs, icons
- Build once, done forever

The decision: **"Do you want to write it twice?"** - Building the dataminer is more work upfront but avoids rewriting the entire schema, APIs, graph algorithms, and frontend when inevitably needing the full dataset.

User context: Built SWTOR backend middleware in Erlang (2007-09), has domain expertise in game data systems. This is a passion project, not a startup - no rush, build it right.

## Research Findings

### Data Available (from Jedipedia)

| Type | Count |
|------|-------|
| Quests | 7,498 |
| Abilities | 32,391 |
| Items | 118,965 |
| NPCs | 42,021 |

Example quest: Global ID `16141008193562682964`, FQN `qst.exp.seasons.01.ep_01_the_hunt`

Quest data includes: objectives, steps, conversations, NPCs, patch versions, linked content.

### File Format Specifications

Found in [bespin](https://github.com/kbatten/bespin) (Python, 2012):

**MYP Format** (.tor archives) - documented in `myp.py`:
- 40-byte header: "MYP" magic + file tables
- File table chain with 12-byte headers
- 34-byte file entries: position, sizes, hash, compression flag
- Compression: 0=none, 1=zlib

**PBUK Format** (containers) - documented in `pbuk.py`:
- 12-byte header: "PBUK" + chunk count + initial size
- Multiple chunks, each with 4-byte size prefix
- Contains DBLB chunks

**DBLB Format** (GOM - Game Object Model) - the key format:
- 8-byte header: "DBLB" + 4 unknown bytes
- Objects with 42-byte headers: size, data type, offset
- Type marker at byte 45: type 15 = zlib compressed
- Each object has: label string + compressed XML payload

**XML Structure** (game objects):
```xml
<Quest GUID="16141008193562682964" fqn="qst.exp.seasons.01.ep_01_the_hunt" Version="1" Revision="42">
  <NameList>...</NameList>
  <ObjectiveList>...</ObjectiveList>
</Quest>
```

### Tools That Exist

- **extracTOR** / **EasyMYP** / **tor-reader** - Extract .tor archives (MYP format)
- **bespin** - Python 2, 2012, parses PBUK/DBLB/XML to SQLite
- **Jedipedia File Reader** - Web tool, no public code

### What Doesn't Exist

- No modern GOM parser
- No Rust implementation
- No public DBLB specification (only bespin's code)

## Architecture

### Pipeline

```
~/swtor/assets/*.tor     (game files)
        ↓
   kessel (Rust)         (this tool)
        ↓
   raw.sqlite            (everything extracted)
        ↓
   filter tools          (SQL queries, scripts)
        ↓
   clean.sqlite          (curated data)
        ↓
   wrangler d1 execute   (push to production)
        ↓
   Cloudflare D1         (huttspawn database)
```

Extraction is a solved problem once built. Curation becomes the iterative part.

### The Crud Problem

Raw game data contains garbage:
- Test/QA content (`test.*`, `qa.*`, `deprecated.*` FQNs)
- 13 years of removed content
- Internal dev placeholders
- Multiple versions (difficulty variants, localization)
- Hidden/unused objects

**Solution**: Separate extraction from curation. Parse everything to raw.sqlite, then build filter queries iteratively until data looks clean. Jedipedia's 7,498 quests (from 118k+ raw objects) shows the curation ratio.

## Current Implementation

### Files Created

```
tools/kessel/
├── Cargo.toml          # Dependencies: flate2, quick-xml, rusqlite, clap, etc.
├── README.md           # This file
└── src/
    ├── main.rs         # CLI entry point
    ├── myp.rs          # MYP archive reader
    ├── pbuk.rs         # PBUK/DBLB parser
    ├── xml_parser.rs   # XML → JSON conversion
    ├── db.rs           # SQLite output
    └── schema/
        └── mod.rs      # GameObject struct
```

### Status

- **Scaffolded**: All modules written
- **Not compiled**: Need Rust in PATH (restart session)
- **Not tested**: Need actual .tor files to test against

## Next Steps

1. **Build kessel**: `cargo build --release`
2. **Test with real .tor files**: Point at SWTOR install
3. **Validate output**: Compare extracted quest count to Jedipedia's 7,498
4. **Build filter queries**: Identify FQN patterns for real vs test content
5. **Schema mapping**: Map extracted fields to huttspawn's D1 schema
6. **Wrangler sync**: Push curated data to D1

## Usage

```bash
# Build
cargo build --release

# Run
./target/release/kessel --input ~/swtor/assets --output raw.sqlite

# With verbose logging
./target/release/kessel -i ~/swtor/assets -o raw.sqlite -v
```

## Output Schema

```sql
-- All extracted objects
CREATE TABLE objects (
    guid TEXT PRIMARY KEY,      -- Game's unique ID
    fqn TEXT NOT NULL,          -- Fully qualified name
    kind TEXT NOT NULL,         -- Object type (Quest, Ability, Item, Npc)
    version INTEGER,
    revision INTEGER,
    json TEXT NOT NULL          -- Full object data as JSON
);

-- Convenience views
CREATE VIEW quests AS SELECT * FROM objects WHERE kind = 'Quest' OR fqn LIKE 'qst.%';
CREATE VIEW abilities AS SELECT * FROM objects WHERE kind = 'Ability' OR fqn LIKE 'abl.%';
CREATE VIEW items AS SELECT * FROM objects WHERE kind = 'Item' OR fqn LIKE 'itm.%';
CREATE VIEW npcs AS SELECT * FROM objects WHERE kind = 'Npc' OR fqn LIKE 'npc.%';
```

## Example Filter Queries

```sql
-- Find real class story quests
SELECT * FROM quests
WHERE fqn LIKE 'qst.class.%'
  AND fqn NOT LIKE '%test%'
  AND fqn NOT LIKE '%deprecated%';

-- Count by kind
SELECT kind, COUNT(*) FROM objects GROUP BY kind ORDER BY COUNT(*) DESC;

-- Find quests with objectives
SELECT guid, fqn, json_extract(json, '$.Quest.ObjectiveList') as objectives
FROM quests
WHERE objectives IS NOT NULL;
```

## References

- **bespin**: https://github.com/kbatten/bespin (format specs)
- **tor-reader**: https://github.com/SWTOR-Slicers/tor-reader (MYP reading)
- **SWTOR-Slicers**: https://github.com/SWTOR-Slicers (community tools)
- **Jedipedia**: https://swtor.jedipedia.net (reference database)
- **TORCommunity**: https://torcommunity.com/database (alternative database)

## Credits

Format specifications derived from [bespin](https://github.com/kbatten/bespin) by kbatten (2012).
