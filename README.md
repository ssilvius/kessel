# Kessel

SWTOR data miner - extracts game objects, strings, and icons from .tor archives.

Named after the spice mines of Kessel, continuing the Star Wars mining theme from [bespin](https://github.com/kbatten/bespin).

## What It Does

Kessel reads SWTOR's `.tor` archive files and extracts:

- **Game Objects** -> SQLite database (176k+ quality-filtered objects)
- **Strings** -> Localized text from STB files (557k+ strings)
- **Icons** -> DDS to WebP conversion with game_id naming
- **Item Classification** -> FQN fragment parsing for structured lookups (gifts module)

### Grammar Processing

Descriptions are automatically cleaned using embedded grammar rules:
- Removes SWTOR template syntax (`<<1>>`, `<<N[singular|plural]>>`)
- Cleans redundant phrasing
- Rules defined in `grammar.toml`, embedded at compile time

## Extraction Results

Quality-filtered extraction (see `docs/STATUS.md` for details):

| Type | Count | Notes |
|------|-------|-------|
| Items | 98,692 | Gear, mods, tacticals, consumables |
| NPCs | 36,242 | Companions, quest NPCs, vendors |
| Quests | 11,692 | Story, side, daily quests |
| Abilities | 2,893 | Class, companion, legacy abilities |
| **Total Objects** | **176,112** | |
| Strings | 557,325 | Localized text (en-us) |
| Icons | 900+ | WebP format, named by game_id |

Object types kept (by FQN prefix): `abl`, `itm`, `npc`, `schem`, `qst`, `cdx`, `ach`, `mpn`, `tal`

## Usage

```bash
# Build
cd tools/kessel
cargo build --release

# Extract game objects to SQLite
# Hashes file auto-downloads from Jedipedia if not found
./target/release/kessel \
  --input ~/swtor/assets \
  --output ~/swtor/data/spice.sqlite

# Extract with icons
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
```

### CLI Options

| Flag | Description |
|------|-------------|
| `-i, --input <DIR>` | Directory containing .tor files (required) |
| `-o, --output <PATH>` | Output SQLite database path (default: raw.sqlite) |
| `-H, --hashes <FILE>` | Hash dictionary file (auto-downloads from Jedipedia if not found) |
| `--icons` | Extract icons to WebP format |
| `--icons-output <DIR>` | Output directory for icons (default: ./icons) |
| `--unfiltered` | Extract all objects without content filtering (filter in ETL instead) |
| `-v, --verbose` | Verbose output |
| `--unknowns <FILE>` | Output file for unknown patterns (JSONL) |

### Unfiltered Mode

By default, Kessel applies content-based filters to skip internal objects:
- NPC abilities (`abl.npc.*`, `abl.operation.*`, etc.)
- Internal items (`itm.loot.*`, `itm.npc.*`, etc.)
- Blueprint NPCs (`npc.blueprints.*`, etc.)

Use `--unfiltered` to extract everything and filter in ETL scripts instead:

```bash
./target/release/kessel \
  --input ~/swtor/assets \
  --output ~/swtor/data/spice.unfiltered.sqlite \
  --hashes ~/swtor/data/hashes_filename.txt \
  --unfiltered
```

This is useful when:
- Investigating missing objects (abilities, items, etc.)
- Creating a complete reference database
- Debugging extraction issues

The `--unfiltered` flag still applies:
- Versioned duplicate filter (FQN with `/`)
- Test/debug/deprecated content filter
- Prefix type filter: only known prefixes are extracted
  (`abl`, `tal`, `itm`, `npc`, `schem`, `qst`, `cdx`, `ach`, `mpn`, `pkg`, `loot`, `rew`, `cnv`, `apc`, `class`)

**Prefix types**:
- `abl` - Abilities (player and NPC)
- `tal` - Talents (discipline tree choices)
- `itm` - Items (equipment, consumables)
- `npc` - NPCs (companions, enemies, vendors)
- `qst` - Quests/Missions
- `cdx` - Codex entries
- `ach` - Achievements
- `schem` - Schematics (crafting)
- `mpn` - Map pins/points
- `pkg` - Packages (bundles)
- `loot` - Loot tables
- `rew` - Rewards
- `cnv` - Conversations
- `apc` - Appearances (character customization)
- `class` - Class definitions

## Icon Extraction

Icons are extracted from DDS files in the .tor archives:

1. **Source**: `/resources/gfx/icons/*.dds` (DXT1/BC1 compressed, 52x52px)
2. **Matching**: Case-insensitive match between file basename and object `icon_name`
3. **Output**: WebP format, named by `game_id` (e.g., `c27c91eabaf927f3.webp`)
4. **Organization**: Subdirectories by object kind (`abilities/`, `items/`, `talents/`, etc.)

Icons are saved for ALL objects that reference them (shared icons get multiple copies with different game_ids).

## FQN Classification

The `gifts` module parses item FQNs into structured fragments that match the frontend TypeScript types exactly. This is the pattern for classifying items by FQN — more modules will follow as we need them.

**Gift items** (`itm.companion.gift.*`):
- FQN: `itm.companion.gift.{type}.{quality}_rank{rank}_v1`
- Rust: `GiftType`, `GiftQuality` enums + `parse_gift_fqn()`
- TS: `GiftType`, `GiftQuality` types in `data/gift-calculator.ts`
- Map: `type → quality → rank → game_id` (Rust `GiftGameIdMap`, TS `GIFT_GAME_IDS`)

Both sides parse the same FQN fragments into the same structure. Icons resolve to `{game_id}.webp`.

## Binary Format Specifications

SWTOR uses a layered container format. Understanding this is key to extracting game data.

### Overview

```
.tor file (MYP archive)
  -> Contains many files, identified by hash
       -> PBUK container (game object bundles)
             -> DBLB wrapper (16 bytes)
                   -> DBLB object block
                         -> Individual GOM objects
                               -> 42-byte header + FQN + ZSTD payload
```

### MYP Archive Format (.tor files)

MYP is BioWare's archive format. Each `.tor` file contains thousands of compressed files.

```
Header (40 bytes):
  bytes 0-3:   Magic "MYP\0"
  bytes 4-7:   Version
  bytes 8-15:  File table offset (u64)
  bytes 16-23: File table size
  bytes 24-31: File count
  bytes 32-39: Reserved

File Table Entry (34 bytes each):
  bytes 0-7:   Data offset in archive (u64)
  bytes 8-11:  Compressed size (u32)
  bytes 12-15: Uncompressed size (u32)
  bytes 16-23: Filename hash (u64) - used for lookup
  bytes 24-27: CRC32
  bytes 28-31: Compression type (0=none, 1=zlib, 2=zstd)
  bytes 32-33: Flags
```

Files are identified by a 64-bit hash of their path. A hash dictionary (`hashes_filename.txt`) maps hashes back to paths.

### PBUK Container Format

PBUK ("Package Bundle"?) wraps collections of game objects.

```
PBUK Header (12 bytes):
  bytes 0-3:   Magic "PBUK"
  bytes 4-5:   Chunk count (u16) - typically 2
  bytes 6-7:   Unknown (u16)
  bytes 8-11:  Offset to first DBLB (always 12)

At offset 12: DBLB Wrapper (16 bytes)
At offset 28: Object DBLB block
```

### DBLB Format (Game Object Model)

DBLB ("Database Block"?) contains the actual game objects.

```
DBLB Wrapper (16 bytes, at PBUK offset 12):
  bytes 0-3:   Magic "DBLB"
  bytes 4-7:   Version (u32, typically 2)
  bytes 8-11:  Padding (zeros)
  bytes 12-15: Total DBLB size (u32)

Object DBLB (at PBUK offset 28):
  bytes 0-3:   Magic "DBLB"
  bytes 4-7:   Version (u32)
  bytes 8-11:  First object size (u32) - important!
  bytes 12-15: Padding
  bytes 16+:   Object data begins
```

### GOM Object Format

Each object within a DBLB block has this structure:

```
Object Structure:
  bytes 0-7:   GUID (u64, little-endian)
  bytes 8-41:  Header data (GUIDs, offsets, flags)
  byte 42+:    FQN string (null-terminated ASCII)
               Example: "itm.gen.lots.weapon.blaster_rifle..."
  [padding]:   Align to next boundary
  [ZSTD]:      Compressed payload (magic: 0x28 0xB5 0x2F 0xFD)
  [8 bytes]:   Footer (next object link)
```

**Key insight**: The ZSTD frame ends 8 bytes before the next object. You must trim the last 8 bytes to get a valid ZSTD frame.

### ZSTD Payload

The compressed payload contains binary GOM data with:
- Length-prefixed strings (1-byte length + ASCII)
- Nested object references
- Property values

```
String format in payload:
  byte 0:      Length (0-255)
  bytes 1-N:   ASCII string data
```

### Parsing Strategy

1. **Open .tor archive** - Read MYP header and file table
2. **Find PBUK files** - Check first 4 bytes for "PBUK" magic
3. **Locate Object DBLB** - Always at offset 28 in PBUK
4. **Read first object size** - From DBLB header bytes 8-11
5. **Parse objects iteratively**:
   - Read 42-byte header
   - Find null-terminated FQN
   - Locate ZSTD magic (0x28 0xB5 0x2F 0xFD)
   - Try decompressing with increasing frame sizes
   - On success, object ends 8 bytes after ZSTD frame
   - Align to 8-byte boundary for next object

### FQN Prefixes

Common Fully Qualified Name prefixes:

| Prefix | Type |
|--------|------|
| `qst.` | Quest |
| `abl.` | Ability |
| `itm.` | Item |
| `npc.` | NPC |
| `cnv.` | Conversation |
| `cdx.` | Codex |
| `ach.` | Achievement |
| `enc.` | Encounter |
| `loc.` | Location |
| `mpn.` | Mission/Planet |
| `dyn.` | Dynamic |
| `spn.` | Spawn |
| `plc.` | Placeable |
| `cbt.` | Combat |
| `veh.` | Vehicle |
| `mtx.` | Cartel Market |
| `tal.` | Talent |

## Project Structure

```
tools/kessel/
+-- Cargo.toml
+-- README.md
+-- grammar.toml        # Description cleanup rules (embedded at compile time)
+-- docs/
|   +-- STATUS.md       # Current extraction results
|   +-- MAPPINGS.md     # File format mappings reference
+-- src/
|   +-- main.rs         # CLI + quality filters
|   +-- lib.rs          # Library exports
|   +-- myp.rs          # MYP archive reader
|   +-- pbuk.rs         # PBUK/DBLB/ZSTD parser
|   +-- stb.rs          # STB string table parser
|   +-- hash.rs         # Hash dictionary loader
|   +-- db.rs           # SQLite output
|   +-- dds.rs          # DDS to WebP conversion
|   +-- gifts.rs        # Gift item FQN classification (shared with TS)
|   +-- grammar.rs      # Description grammar processor
|   +-- unknowns.rs     # Unknown pattern tracking
|   +-- schema/
|       +-- mod.rs      # GameObject struct
+-- tests/              # Integration tests
```

## Output Schema

```sql
CREATE TABLE objects (
    guid INTEGER PRIMARY KEY,
    fqn TEXT NOT NULL UNIQUE,
    game_id TEXT NOT NULL,      -- sha256(fqn:guid)[0:16]
    kind TEXT NOT NULL,
    icon_name TEXT,
    string_id INTEGER,
    for_export INTEGER DEFAULT 1,
    version INTEGER DEFAULT 0,
    revision INTEGER DEFAULT 0,
    json TEXT,                  -- Parsed payload data
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE strings (
    fqn TEXT PRIMARY KEY,
    locale TEXT NOT NULL,
    id1 INTEGER NOT NULL,
    id2 INTEGER NOT NULL,
    text TEXT NOT NULL,
    version INTEGER DEFAULT 0
);

-- Views for common queries
CREATE VIEW abilities AS SELECT * FROM objects WHERE kind = 'Ability' OR fqn LIKE 'abl.%';
CREATE VIEW items AS SELECT * FROM objects WHERE kind = 'Item' OR fqn LIKE 'itm.%';
CREATE VIEW npcs AS SELECT * FROM objects WHERE kind = 'Npc' OR fqn LIKE 'npc.%';
CREATE VIEW quests AS SELECT * FROM objects WHERE kind = 'Quest' OR fqn LIKE 'qst.%';

CREATE INDEX idx_objects_kind ON objects(kind);
CREATE INDEX idx_objects_icon_name ON objects(icon_name);
CREATE INDEX idx_strings_locale ON strings(locale);
CREATE INDEX idx_strings_id2 ON strings(id2);
```

## Dependencies

- `zstd` - ZSTD decompression (current SWTOR format)
- `flate2` - zlib decompression (legacy format)
- `rusqlite` - SQLite output
- `image` + `image_dds` - DDS to WebP conversion
- `quick-xml` + `serde_json` - XML parsing
- `regex` - Grammar rule processing
- `toml` - Grammar configuration parsing
- `ureq` - HTTP client (auto-downloading hash dictionary)

## References

- [bespin](https://github.com/kbatten/bespin) - Original format specs (Python, 2012)
- [Jedipedia](https://swtor.jedipedia.net) - Reference database
- [SWTOR-Slicers](https://github.com/SWTOR-Slicers) - Community tools
