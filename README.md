# Kessel

SWTOR data miner -- extracts game objects, localized strings, and icons from `.tor` archives into SQLite.

Named after the spice mines of Kessel.

## What it does

Kessel reads SWTOR's `.tor` archive files and produces:

- **SQLite database** with 176k+ game objects (abilities, items, NPCs, quests, talents, etc.)
- **557k+ localized strings** extracted from STB string tables
- **WebP icons** converted from DDS textures, named by deterministic game_id

Descriptions are automatically cleaned via embedded grammar rules that strip SWTOR's template syntax (`<<1[%d seconds/%d second/%d seconds]>>` becomes natural English).

## Usage

```bash
cargo build --release

# Extract game objects and strings
./target/release/kessel \
  --input ~/swtor/Assets \
  --output spice.sqlite

# With icon extraction
./target/release/kessel \
  --input ~/swtor/Assets \
  --output spice.sqlite \
  --icons \
  --icons-output ./icons

# Unfiltered (skip content filters, keep everything)
./target/release/kessel \
  --input ~/swtor/Assets \
  --output spice.sqlite \
  --unfiltered
```

The hash dictionary auto-downloads from Jedipedia on first run.

### CLI flags

| Flag | Description |
|------|-------------|
| `-i, --input <DIR>` | SWTOR Assets directory (required) |
| `-o, --output <PATH>` | SQLite output path (default: `raw.sqlite`) |
| `-H, --hashes <FILE>` | Hash dictionary (auto-downloads if missing) |
| `--icons` | Enable DDS to WebP icon extraction |
| `--icons-output <DIR>` | Icon output directory (default: `./icons`) |
| `--unfiltered` | Skip content filters (keeps NPC abilities, internal items, etc.) |
| `-v, --verbose` | Verbose logging |
| `--unknowns <FILE>` | Track unknown patterns to JSONL |

### Filtering

By default, kessel applies content filters to skip internal/NPC objects. Use `--unfiltered` to extract everything.

Both modes always skip: versioned duplicates (FQN containing `/`), test/debug/deprecated content.

Extracted FQN prefixes: `abl` (abilities), `tal` (talents), `itm` (items), `npc` (NPCs), `qst` (quests), `cdx` (codex), `ach` (achievements), `schem` (schematics), `mpn` (map pins), `pkg` (packages), `loot` (loot tables), `rew` (rewards), `cnv` (conversations), `apc` (appearances), `class` (class definitions).

## Output schema

```sql
CREATE TABLE objects (
    guid TEXT PRIMARY KEY,
    fqn TEXT NOT NULL,
    game_id TEXT NOT NULL,      -- sha256(fqn:guid)[0:16]
    kind TEXT NOT NULL,
    icon_name TEXT,
    string_id INTEGER,          -- links to strings.id2
    for_export INTEGER DEFAULT 1,
    version INTEGER DEFAULT 0,
    revision INTEGER DEFAULT 0,
    json TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
);

CREATE TABLE strings (
    fqn TEXT PRIMARY KEY,
    locale TEXT NOT NULL,
    id1 INTEGER NOT NULL,
    id2 INTEGER NOT NULL,       -- links to objects.string_id
    text TEXT NOT NULL,
    version INTEGER DEFAULT 0
);
```

Views: `abilities`, `items`, `npcs`, `quests`.

### Querying

```bash
# Object counts by type
sqlite3 spice.sqlite "SELECT kind, COUNT(*) FROM objects GROUP BY kind ORDER BY 2 DESC;"

# Find an ability by name
sqlite3 spice.sqlite "
  SELECT o.fqn, s.text
  FROM objects o
  JOIN strings s ON s.id2 = o.string_id AND s.locale = 'en-us' AND s.id1 = 1
  WHERE o.kind = 'Ability' AND s.text LIKE '%Ravage%';
"
```

## How it works

SWTOR stores game data in layered binary containers:

```
.tor archive (MYP format)
  -> Files identified by 64-bit hash
    -> PBUK containers (game object bundles)
      -> DBLB blocks (database blocks)
        -> GOM objects (42-byte header + FQN + ZSTD-compressed payload)
```

Kessel reads the archive, resolves hashes to paths via a dictionary, decompresses each layer (zstd or zlib), and extracts structured data from the binary GOM payloads.

String tables (`.stb` files) are parsed separately and linked to objects via `string_id`.

Icons are DDS textures converted to lossless WebP, organized by object kind (`abilities/`, `items/`, `talents/`, etc.) and named by `game_id` for deterministic frontend lookup.

## Project structure

```
kessel/
  Cargo.toml
  grammar.toml          # Description cleanup rules (embedded at compile time)
  src/
    main.rs             # CLI entry point and extraction pipeline
    lib.rs              # Library exports
    myp.rs              # MYP archive reader (decompress .tor files)
    pbuk.rs             # PBUK/DBLB parser (extract GOM objects)
    stb.rs              # STB string table parser
    db.rs               # SQLite database (batch inserts, grammar application)
    dds.rs              # DDS to WebP icon conversion
    hash.rs             # Hash dictionary and game_id computation
    grammar.rs          # Template/literal/cleanup rule processor
    gifts.rs            # Gift item FQN classification
    unknowns.rs         # Unknown pattern tracking
    schema/mod.rs       # GameObject struct and binary extraction
    bin/
      analyze_headers.rs  # DDS header analysis utility
  tests/
    schema_test.rs
    pbuk_test.rs
    myp_test.rs
    xml_parser_test.rs
```

## License

MIT
