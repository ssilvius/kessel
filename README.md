# Kessel

SWTOR data miner - extracts game objects and icons from .tor archives.

Named after the spice mines of Kessel, continuing the Star Wars mining theme from [bespin](https://github.com/kbatten/bespin).

## What It Does

Kessel reads SWTOR's `.tor` archive files and extracts:

- **Game Objects** → SQLite database (265,664 objects extracted)
- **Icons** → PNG files named by hash (69,553 icons extracted)

## Extraction Results

### Game Objects

Object types by FQN prefix:
- `qst.*` - Quests
- `abl.*` - Abilities
- `itm.*` - Items
- `npc.*` - NPCs
- `cnv.*` - Conversations
- `cdx.*` - Codex entries
- `ach.*` - Achievements
- And many more...

### Icons

Icons are named by their hash (e.g., `C28BE968F7F1543C.png`) for CDN deployment. A mapping file links icon names to hashes.

## Binary Format Specifications

SWTOR uses a layered container format. Understanding this is key to extracting game data.

### Overview

```
.tor file (MYP archive)
  └── Contains many files, identified by hash
        └── PBUK container (game object bundles)
              └── DBLB wrapper (16 bytes)
                    └── DBLB object block
                          └── Individual GOM objects
                                └── 42-byte header + FQN + ZSTD payload
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

### Icon Files

Icons are stored as DDS (DirectDraw Surface) files:
- Format: DXT1 (BC1) compressed
- Typical size: 52x52 pixels
- Path pattern: `/resources/gfx/icons/*.dds`
- Found in: `swtor_main_gfx_*.tor` archives

## Usage

```bash
# Build
cargo build --release

# Extract game objects to SQLite
./target/release/kessel \
  --input ~/swtor/assets \
  --output ~/swtor/data/kessel.sqlite \
  --hashes ~/swtor/data/hashes_filename.txt

# Extract icons (example script)
cargo run --release --example extract_icons
```

## Project Structure

```
tools/kessel/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs         # CLI
│   ├── lib.rs          # Library exports
│   ├── myp.rs          # MYP archive reader
│   ├── pbuk.rs         # PBUK/DBLB parser
│   ├── xml_parser.rs   # XML → JSON
│   ├── db.rs           # SQLite output
│   └── schema/
│       └── mod.rs      # GameObject struct
└── examples/
    ├── extract_icons.rs    # Bulk icon extraction
    ├── extract_icon.rs     # Single icon test
    ├── debug_header.rs     # Binary structure debugging
    └── query_db.rs         # Database queries
```

## Output Schema

```sql
CREATE TABLE objects (s
    guid TEXT PRIMARY KEY,
    fqn TEXT NOT NULL,
    kind TEXT NOT NULL,
    version INTEGER,
    revision INTEGER,
    json TEXT NOT NULL
);

CREATE INDEX idx_objects_fqn ON objects(fqn);
CREATE INDEX idx_objects_kind ON objects(kind);
```

## Dependencies

- `zstd` - ZSTD decompression (current SWTOR format)
- `flate2` - zlib decompression (legacy format)
- `rusqlite` - SQLite output
- `image` + `image_dds` - DDS → PNG conversion
- `quick-xml` + `serde_json` - XML parsing

## References

- [bespin](https://github.com/kbatten/bespin) - Original format specs (Python, 2012)
- [Jedipedia](https://swtor.jedipedia.net) - Reference database
- [SWTOR-Slicers](https://github.com/SWTOR-Slicers) - Community tools
