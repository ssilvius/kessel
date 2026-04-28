# Kessel CLI

## Synopsis

```
kessel --input <DIR> --output <PATH> [OPTIONS]
```

## Required flags

| Flag | Description |
|------|-------------|
| `-i, --input <DIR>` | SWTOR Assets directory containing `.tor` files |
| `-o, --output <PATH>` | SQLite output path (default: `raw.sqlite`) |

## Optional flags

| Flag | Description |
|------|-------------|
| `-H, --hashes <FILE>` | Hash dictionary path. Auto-downloads from Jedipedia if not provided. The downloaded file is saved alongside the input directory as `hashes_filename.txt`. |
| `--icons` | Enable DDS-to-WebP icon extraction. |
| `--icons-output <DIR>` | Icon output directory (default: `./icons`). Has no effect without `--icons`. |
| `--unfiltered` | Skip content filters. See [Filtering](#filtering) below. |
| `--unknowns <FILE>` | Write unrecognized payload patterns to a JSONL file for analysis. |
| `-v, --verbose` | Print additional debug output. |

## Examples

```bash
# Minimal — hash dictionary auto-downloads
./target/release/kessel -i ~/swtor/Assets -o spice.sqlite

# With icon extraction
./target/release/kessel \
  -i ~/swtor/Assets \
  -o spice.sqlite \
  --icons \
  --icons-output ./icons

# Explicit hash dictionary
./target/release/kessel \
  -i ~/swtor/Assets \
  -o spice.sqlite \
  -H ~/swtor/data/hashes_filename.txt

# Unfiltered (keep everything, including NPC abilities and internal items)
./target/release/kessel \
  -i ~/swtor/Assets \
  -o spice.sqlite \
  --unfiltered
```

## Filtering

By default kessel applies content filters to skip objects that aren't useful for player-facing tools:

- Internal/NPC-only abilities (FQNs containing `npc`, `world_design`, `test`, `deprecated`, etc.)
- Items flagged as internal or not for sale
- Objects with no localized strings

Use `--unfiltered` to bypass these filters. Both modes always:

- Skip versioned FQN duplicates — multiple `/major/minor` variants of the same ability are collapsed to one row, first-encountered wins
- Skip objects with empty FQNs

### Extracted FQN prefixes (filtered mode)

`abl` abilities · `tal` talents · `itm` items · `npc` NPCs · `qst` quests · `mpn` phases · `cdx` codex · `ach` achievements · `cnv` conversations · `enc` encounters · `spn` spawns · `plc` placeables

## Icons

When `--icons` is set, kessel extracts DDS textures from the archive and converts them to lossless WebP. Icons are organized by object kind:

```
icons/
  abilities/   {game_id}.webp
  items/       {game_id}.webp
  talents/     {game_id}.webp
  ...
```

The filename is always the object's `game_id` — a deterministic 16-character hex string (`sha256(fqn:guid)[0:16]`). This is the stable name to use for frontend CDN paths.

For abilities whose GOM payloads don't embed an icon reference (base-class shared abilities recovered from versioned FQNs), `icon_overrides.toml` provides a compile-time FQN→icon_name mapping.

## Utilities

### dump-npp

Debug binary for dumping GOM objects matching a prefix or exact FQN:

```bash
cargo run --bin dump-npp -- \
  -i ~/swtor/Assets \
  -H ~/swtor/data/hashes_filename.txt \
  -p npp \        # prefix filter (default: npp)
  -n 20 \         # result limit (default: 20)
  -f abl.sith_warrior.force_charge   # exact FQN (can repeat; overrides -p)
```

Prints header bytes, payload hex, and extracted strings for each matching object.
