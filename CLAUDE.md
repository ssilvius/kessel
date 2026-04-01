# Kessel

SWTOR game data extractor. Parses binary `.tor` archives and outputs structured data to SQLite.

## Build and test

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Run extraction

```bash
# Minimum (hash dictionary auto-downloads from Jedipedia)
./target/release/kessel -i ~/swtor/Assets -o spice.sqlite

# With icons
./target/release/kessel -i ~/swtor/Assets -o spice.sqlite --icons --icons-output ./icons

# Unfiltered (no content filters, keeps NPC/internal objects)
./target/release/kessel -i ~/swtor/Assets -o spice.sqlite --unfiltered
```

## Architecture

Binary format layers: `.tor` (MYP archive) -> PBUK container -> DBLB block -> GOM objects (ZSTD compressed).

Data flow:
1. `myp.rs` reads .tor archives, decompresses entries (zstd/zlib)
2. `hash.rs` resolves 64-bit file hashes to paths via Jedipedia dictionary
3. `pbuk.rs` extracts GOM objects from PBUK/DBLB containers
4. `schema/mod.rs` converts binary GOM objects to structured `GameObject` (GUID, FQN, game_id, kind, icon_name, string_id)
5. `stb.rs` extracts localized strings from STB string tables
6. `grammar.rs` cleans SWTOR template syntax from descriptions (rules in `grammar.toml`)
7. `db.rs` batch-inserts objects and strings to SQLite
8. `dds.rs` converts DDS textures to WebP icons, matched to objects by name

## Key concepts

- **FQN** (Fully Qualified Name): dot-separated object identifier like `abl.sith_warrior.skill.rage.ravage`. The prefix determines the object kind.
- **game_id**: deterministic identifier `sha256(fqn:guid)[0:16]`. Used for icon filenames and frontend lookups.
- **string_id**: links objects to their localized strings in STB tables via `objects.string_id = strings.id2`.
- **Grammar rules**: embedded at compile time from `grammar.toml`. Template rules handle `<<N[...]>>` patterns, literal rules do exact replacements, cleanup rules are regex post-processing.

## Code conventions

- No `unwrap()` in library code -- use `anyhow` for error propagation
- Batch database inserts (flush at 5000 items)
- All hashing uses SHA-256 truncated to 16 hex chars
- Icon IDs must match the frontend `computeIconId()` function
- JSON for all data interchange (no TOML/YAML for data)

## Known gaps

- NODE files (`.node` prototypes in `/resources/systemgenerated/prototypes/`) are not yet parsed. Player abilities live here, not in bucket files.
- GSF talent descriptions need stat value extraction (firing arc degrees, tracking penalty %, etc.)
- No automated test for full extraction pipeline (needs SWTOR assets)
