# Kessel

SWTOR game data extractor. Parses binary `.tor` archives and outputs structured data to SQLite.

## Workspace layout

Cargo workspace with two crates:

- `kessel/` -- the production crate (the `kessel` binary + library). This is what ships.
- `kessel-discovery/` -- one-off reverse-engineering tools, probes, dumps, scanners. Not shipped by default; built on demand for discovery work.

## Build and test

```bash
# Default: only builds production `kessel` binary
cargo build --release
cargo test -p kessel
cargo clippy -p kessel -- -D warnings
cargo fmt --check

# Full workspace (includes ~60 discovery binaries)
cargo build --release --workspace
cargo build --release -p kessel-discovery --bin <name>  # one-off recon tool
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

- GSF talent stat extraction covers 250/350 talents (#80, gsf_talent_stats
  table). The remaining ~100 are flag-only talents whose effects live on the
  parent ability or in script hooks; surfacing those would require
  parent-ability stat-block parsing as a separate pass.
- No automated test for full extraction pipeline (needs SWTOR assets).
- GSF per-component combat math (per-laser damage, range, accuracy, firing
  arc) lives in `swtor.exe`, not in any `.tor` data file. Confirmed via
  exhaustive byte search (legion `019e4cbb`). Recovery path: Sean's in-game
  capture, or swtor.exe decompile (out of kessel scope).
- CC field hash → name dictionary unknown. The 4-byte CC hashes (6F6FAE37
  stringRef, 17E2840B abilityRef, etc.) referenced in MAPPINGS.md are not in
  client.gom -- they live in a separate proprietary Bioware namespace.
  Requires a known-plaintext attack to reverse (spike issue #144).
- SCPT compiled-native scripts (1,196 in
  `/resources/systemgenerated/compilednative/`) contain UI/SFX logic, not
  combat math. Decoder available as `kessel::scpt` (#127) but no consumer
  wired by default.
- Per-property post-CF40 value byte layout (int8/16/32, enum_ref, string,
  array, class_ref) is not yet decoded. The schema-aware walker (#125)
  resolves marker names and emits typed property keys, but value extraction
  is foundation-only on quest_details (#129) and quest_objectives (#130) --
  values are recorded as `"PRESENT"` flags rather than decoded enum members
  or ints. Follow-on PRs lift these per Quest/Ability/Item/Npc class.
