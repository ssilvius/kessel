# SWTOR Data Mappings Reference

This document records the relationships between file types, FQN patterns, and data structures discovered during kessel development.

## Archive → File Type Mapping

| Archive Pattern | Contents | Notes |
|-----------------|----------|-------|
| `swtor_main_*.tor` | GOM objects (PBUK/DBLB) | Abilities, items, NPCs, quests |
| `swtor_en-us_global_*.tor` | English STB strings | ~18MB, main string tables |
| `swtor_en-us_cnv_*.tor` | Conversation audio/strings | Companion chars, flashpoints |
| `swtor_main_gfx_*.tor` | Icons, textures | DDS format |
| `swtor_main_anim_*.tor` | Animations | Not needed for data |

## GOM Object Types (from FQN prefix)

| FQN Prefix | Kind | Count | Description |
|------------|------|-------|-------------|
| `abl.*` | Ability | 56,256 | Player and NPC abilities |
| `itm.*` | Item | 51,777 | Equipment, consumables, etc |
| `spn.*` | Spawn | 31,663 | Spawn points |
| `npc.*` | Npc | 21,193 | Non-player characters |
| `hyd.*` | Hydra | 13,336 | Hydra system objects |
| `plc.*` | Placeable | 8,697 | World objects |
| `epp.*` | epp | 7,981 | Effect/particle definitions |
| `cnd.*` | cnd | 7,952 | Conditions |
| `npp.*` | npp | 7,927 | NPC prototypes |
| `schem.*` | schem | 6,601 | Schematics |
| `dyn.*` | Dynamic | 5,314 | Dynamic objects |
| `qst.*` | Quest | 5,312 | Quest definitions |
| `enc.*` | Encounter | 4,721 | Encounter scripts |
| `ach.*` | Achievement | 2,903 | Achievements |
| `cdx.*` | Codex | 1,465 | Codex entries |

## STB File Distribution

| Path Pattern | Count | Purpose |
|--------------|-------|---------|
| `/str/cnv/` | ~15,000 | Conversation dialogue (text only) |
| `/str/gui/` | ~200 | UI strings |
| `/str/abl/` | 14 | Ability strings (sparse!) |
| `/str/itm/` | ~10 | Item strings |
| `/str/misc/` | ~50 | Mail, system messages |
| `/str/sys/` | ~20 | System strings |

## Main String Tables (Root-Level STB Files)

These are the **primary string tables** containing display names and descriptions.
All are at `/resources/en-us/str/*.stb`.

| Hash | File | FQN Prefix | GOM Kind | Notes |
|------|------|------------|----------|-------|
| `8154956D#54305B3B` | **abl.stb** | `str.abl` | Ability | **Main ability names/descriptions** |
| `32EF86D8#FB5DCB98` | **itm.stb** | `str.itm` | Item | Main item names/descriptions |
| `B2A26731#0DDC0C21` | **npc.stb** | `str.npc` | Npc | NPC names |
| `D0974307#8ED12356` | **qst.stb** | `str.qst` | Quest | Quest names/descriptions |
| `DE4E4A7A#D981F52E` | **mpn.stb** | `str.mpn` | Quest | Mission/planet names |
| `E4889EE0#10A7098C` | **cdx.stb** | `str.cdx` | Codex | Codex entry text |
| `8BBE1958#45F4B329` | **ach.stb** | `str.ach` | Achievement | Achievement names |
| `0A0AA3B0#C9A1A82E` | **plc.stb** | `str.plc` | Placeable | Placeable object names |
| `395695A3#FB71F0E0` | **cnd.stb** | `str.cnd` | cnd | Condition strings |
| `3464203D#44C36773` | **dec.stb** | `str.dec` | - | Decoration names |
| `20F826C1#846BCEB6` | **apt.stb** | `str.apt` | - | Apartment/stronghold names |
| `33E7A874#2B4A89C5` | **lky.stb** | `str.lky` | - | Legacy names |
| `93A7AD6F#5DE17C3F` | **tal.stb** | `str.tal` | - | Talents |
| `67D4E926#05A1DBCF` | **nco.stb** | `str.nco` | - | Unknown |
| `88467456#5C47336E` | **pcs.stb** | `str.pcs` | - | Pieces/components |
| `0BDBE097#8D08ACC5` | **rdd.stb** | `str.rdd` | - | Unknown |
| `B3C04BD8#649A4539` | **ahd.stb** | `str.ahd` | - | Unknown |
| `C9B4C51A#E546555D` | **svy.stb** | `str.svy` | - | Survey/exploration |
| `E5756622#2F4C43F4` | **mrp.stb** | `str.mrp` | - | Unknown |

## Secondary STB Files

### Ability Subdirectories (14 files)
| Hash | Path | Purpose |
|------|------|---------|
| `70E13193#455280A7` | `str/abl/agent/skill.stb` | Agent-specific abilities |
| `A613EE4F#7DDC14EE` | `str/abl/player/skill_trees.stb` | Skill tree names |
| `35B23666#065676AC` | `str/abl/player/proficiency.stb` | Proficiency strings |
| `4A0473F2#9D4715B4` | `str/abl/player/mount.stb` | Mount abilities |

### Item Subdirectories (7 files)
| Hash | Path | Purpose |
|------|------|---------|
| `0F2B7CCC#742F3F43` | `str/itm/loot/quality.stb` | Loot quality names |
| `4D815A6D#6AC4BDA7` | `str/itm/modifiers.stb` | Item modifier names |
| `30809B40#7C03CF70` | `str/itm/enhancement/stations.stb` | Enhancement station names |
| `4C005A0C#9661CC02` | `str/itm/enhancement/messages.stb` | Enhancement messages |

### UI/GUI Files (178 files)
| Hash | Path | Purpose |
|------|------|---------|
| `4B0188AB#807E8D17` | `str/gui/disciplinewindow.stb` | Discipline UI |
| `6823CCEE#D32147AB` | `str/gui/skilltree.stb` | Skill tree UI |
| `3490615F#AB8F2C76` | `str/gui/charcreateabilities.stb` | Char create abilities |
| `3BD82CC0#DCACA396` | `str/gui/classnames.stb` | Class names |

## FQN → STB Mapping Strategy

### Object to String Relationship
```
Object FQN:  abl.sith_inquisitor.skill.corruption.innervate
String FQN:  str.abl.sith_inquisitor.skill.corruption.innervate
Join:        strings.fqn = 'str.' || objects.fqn
```

### STB Path to FQN Conversion
```
STB Path:    /resources/en-us/str/abl/agent/skill.stb
FQN Prefix:  str.abl.agent.skill
Locale:      en-us
```

## GUID Structure

### GOM Object GUID
- **Location**: Header bytes 0-7 (little-endian u64)
- **Format**: `E000458329C3673A` (16 hex chars)
- **Example**: `abl.sith_inquisitor.skill.corruption.innervate` → GUID `E000458329C3673A`

### Archive File Hash
- **Format**: `PH#SH#path#CRC` where PH=primary hash, SH=secondary hash
- **Combined**: `(PH << 32) | SH` = 64-bit lookup key
- **Example**: `70E13193#455280A7` → combined `70E13193455280A7`

## Conversation Data Structure

### What STB Files Contain
- Localized dialogue text only
- No metadata about speakers, choices, or consequences

### What STB Files Do NOT Contain
- Light/Dark alignment point values
- Companion affection changes
- Conversation branching logic
- Speaker identification
- Animation/timing data

### Where Structure Data Lives
- `cnv.*` GOM objects (not yet extracted)
- These would be in `swtor_main_*.tor` archives

## String Reference Patterns in GOM Payloads

Abilities reference strings via patterns in their binary payload:
```
"str.abl.sith_inquisitor.skill.corruption"  → STB file reference
"str.abl.agent.skill"                        → STB file reference
```

These appear in the `strings` array extracted from GOM payloads.

## Extraction Priority

### Extraction Filter (Applied in Kessel)

**GOM Objects - KEEP:**
| Prefix | Count | Purpose |
|--------|-------|---------|
| `abl.*` | 56k | Abilities |
| `itm.*` | 52k | Items |
| `npc.*` | 21k | NPCs |
| `qst.*` | 5k | Quests |
| `cdx.*` | 1.5k | Codex |
| `ach.*` | 3k | Achievements |
| **Total** | **~139k** | |

**GOM Objects - SKIP:**
| Prefix | Count | Reason |
|--------|-------|--------|
| `spn.*` | 32k | Spawn points (internal) |
| `hyd.*` | 13k | Hydra system (internal) |
| `plc.*` | 9k | Placeables (world objects) |
| `epp.*` | 8k | Effects/particles |
| `cnd.*` | 8k | Conditions (internal) |
| `npp.*` | 8k | NPC prototypes |
| `schem.*` | 7k | Schematics |
| `dyn.*` | 5k | Dynamic objects |
| `enc.*` | 5k | Encounters |
| Others | ~15k | Various internal |
| **Total** | **~126k** | |

**STB Files - KEEP (6 files):**
- `abl.stb`, `itm.stb`, `npc.stb`, `qst.stb`, `cdx.stb`, `ach.stb`

**STB Files - SKIP (~17k files):**
- All conversation files (`str/cnv/*`)
- All GUI files (`str/gui/*`)
- All subdirectory files

### Future Phases
1. **Conversations** - Extract `cnv.*` GOM objects + link to `str/cnv/` STB dialogue
2. **Icons** - Link icon hashes to abilities/items

## Open Questions

1. **~~Where are most ability names?~~** ✅ RESOLVED
   - Found `/resources/en-us/str/abl.stb` (hash `8154956D#54305B3B`)
   - Main string table for all abilities
   - GOM objects reference it via `str.abl` pattern in payload

2. **How do GOM objects reference specific string IDs?**
   - STB entries have (id1, id2) pairs
   - GOM payloads have `str.abl` references (no specific ID visible)
   - Theory: FQN suffix maps to string ID somehow
   - Example: `abl.agent.snipe` → look up "snipe" in `abl.stb`?
   - Need to parse an STB file to understand entry structure

3. **Conversation structure format**
   - `cnv.*` objects exist but not extracted from main archives
   - Need to identify which archives contain them
   - Conversation text is in `str/cnv/` STB files (17k files)

4. **What archives contain the main STB files?**
   - Likely in `swtor_en-us_global_1.tor` (18MB)
   - Need to verify by extracting
