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

### Kessel v3 Extraction Results (7.8b)

| FQN Prefix | Kind | Count | Description | Extracted |
|------------|------|-------|-------------|-----------|
| `itm.*` | Item | 94,021 | Equipment, consumables, etc | ✅ |
| `npc.*` | Npc | 34,583 | Non-player characters | ✅ |
| `schem.*` | Schematic | 13,774 | Crafting schematics | ✅ |
| `mpn.*` | Quest | 9,852 | Mission points (objectives, waypoints) | ✅ |
| `ach.*` | Achievement | 6,110 | Achievements | ✅ |
| `cdx.*` | Codex | 3,152 | Codex entries | ✅ |
| `abl.*` | Ability | 2,713 | Player abilities (filtered) | ✅ |
| `tal.*` | Talent | 971 | Talents/passives | ✅ |
| `qst.*` | Quest | 278 | Quest definitions | ✅ |
| `loot.*` | Loot | 89 | Loot tables (junk, lockboxes) | ✅ NEW |
| `pkg.*` | Package | 6 | Trainer packages (schematic lists) | ✅ NEW |
| **Total** | | **165,549** | (with quality filters applied) | |

### String Tables (STB)

| FQN Prefix | Count | Purpose |
|------------|-------|---------|
| `str.itm.*` | 241,432 | Item names/descriptions |
| `str.abl.*` | 182,665 | Ability names/descriptions |
| `str.qst.*` | 45,356 | Quest names/descriptions |
| `str.npc.*` | 40,305 | NPC names |
| `str.cdx.*` | 7,665 | Codex entry text |
| `str.tal.*` | 2,341 | Talent/passive names |
| **Total** | **557,325** | |

### Reference Counts (pre-filter)

| FQN Prefix | Kind | Count | Description | Extracted |
|------------|------|-------|-------------|-----------|
| `abl.*` | Ability | 56,256 | Player and NPC abilities | ✅ (filtered to 2,713) |
| `tal.*` | Talent | 970 | Talent/passive modifiers | ✅ |
| `itm.*` | Item | 51,777 | Equipment, consumables, etc | ✅ |
| `npc.*` | Npc | 21,193 | Non-player characters | ✅ |
| `schem.*` | schem | 6,601 | Schematics | ✅ |
| `qst.*` | Quest | 5,312 | Quest definitions | ✅ |
| `ach.*` | Achievement | 2,903 | Achievements | ✅ |
| `cdx.*` | Codex | 1,465 | Codex entries | ✅ |
| `mpn.*` | Mpn | 9,852 | Mission points | ✅ |
| `spn.*` | Spawn | 31,663 | Spawn points | ❌ |
| `hyd.*` | Hydra | 13,336 | Hydra system objects | ❌ |
| `plc.*` | Placeable | 8,697 | World objects | ❌ |
| `epp.*` | epp | 7,981 | Effect/particle definitions | ❌ |
| `cnd.*` | cnd | 7,952 | Conditions | ❌ |
| `npp.*` | npp | 7,927 | NPC prototypes | ❌ |
| `dyn.*` | Dynamic | 5,314 | Dynamic objects | ❌ |
| `enc.*` | Encounter | 4,721 | Encounter scripts | ❌ |

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
| `93A7AD6F#5DE17C3F` | **tal.stb** | `str.tal` | Talent | Talent/passive names ✅ (added 7.8b) |
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

## GOM Header Structure (42 bytes)

Every GOM object has a 42-byte binary header with this structure:

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0-5 | 6 | Object Hash | Unique per object |
| 6-7 | 2 | Magic | Always `00 e0` |
| 8-9 | 2 | Unknown | Always `0f 00` (15) |
| 10-11 | 2 | Size A | Varies |
| 12-13 | 2 | Constant | Always `32 00` (50) |
| 14-15 | 2 | Size B | Size A - 1 |
| 16-19 | 4 | **Type ID** | Unique per GOM class |
| 20-23 | 4 | Flags | Varies |
| 24-29 | 6 | Padding | Always zeros |
| 30-31 | 2 | Block Size | 96 or 104 |
| 32-35 | 4 | Payload Size | Approximate payload bytes |
| 36-37 | 2 | Block Size | Duplicate of 30-31 |
| 38-39 | 2 | Constant | Always `05 00` (5) |
| 40-41 | 2 | Subtype | `01 02` or `01 03` |

### Type IDs (Bytes 16-19)

| Type | Type ID | Subtype | Count (7.8b) |
|------|---------|---------|--------------|
| `tal.*` | `01fb54d9` | `0102` | 970 |
| `abl.*` | `d2f48302` | `0103` | 2,713 |
| `qst.*` | `d2c3de2a` | `0103` | 278 |
| `itm.*` | `0ecd1a01` | `0103` | 93,903 |
| `npc.*` | `bde17800` | `0103` | 34,296 |
| `mpn.*` | `c767e4f9` | `0103` | 9,852 |
| `cdx.*` | `ec397625` | `0102` | 3,152 |
| `ach.*` | `a03ec53a` | `0102` | 6,108 |
| `schem.*` | `8a40a8df` | `0102` | 13,686 |

**Subtype Pattern:**
- `0102` = Static/reference types (talents, codex, achievements, schematics)
- `0103` = World/instance types (abilities, quests, items, NPCs, mission points)

## GUID Structure

### GOM Object GUID
- **Location**: Header bytes 0-5 (6 bytes, unique hash)
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

### Current Filter (Applied in Kessel)

**GOM Objects - KEEP (with quality filters):**
| Prefix | Post-Filter | Purpose |
|--------|-------------|---------|
| `itm.*` | 94,011 | Items (gear, mods, consumables) |
| `npc.*` | 34,582 | NPCs (companions, vendors, quest givers) |
| `schem.*` | 13,773 | Schematics (crafting recipes) |
| `qst.*` | 10,130 | Quests |
| `ach.*` | 6,107 | Achievements |
| `cdx.*` | 3,152 | Codex entries |
| `abl.*` | 2,712 | Abilities (class, companion, legacy) |
| `mpn.*` | 25 | Mission/planet markers |
| **Total** | **164,492** | |

**Quality Filters Applied:**
- Skip versioned duplicates (`abl.foo.bar/17/5` → keep only `abl.foo.bar`)
- Skip test/debug/deprecated content
- Skip 34 internal ability prefixes (npc, qtr, operation, etc.)
- Skip 10 internal item prefixes (has_item, irating, etc.)
- Skip 5 internal NPC prefixes (blueprints, ability, etc.)

**GOM Objects - SKIP:**
| Prefix | Reason |
|--------|--------|
| `spn.*` | Spawn points (internal) |
| `hyd.*` | Hydra system (internal) |
| `plc.*` | Placeables (world objects) |
| `epp.*` | Effects/particles |
| `cnd.*` | Conditions (internal) |
| `npp.*` | NPC prototypes |
| `dyn.*` | Dynamic objects |
| `enc.*` | Encounters |

**STB Files - KEEP (8 root files):**
- `abl.stb`, `tal.stb`, `itm.stb`, `npc.stb`, `qst.stb`, `cdx.stb`, `ach.stb`, `schem.stb`

**STB Files - SKIP (~17k files):**
- All conversation files (`str/cnv/*`)
- All GUI files (`str/gui/*`)
- All subdirectory files

### Future Phases
1. **Conversations** - Extract `cnv.*` GOM objects + link to `str/cnv/` STB dialogue
2. **Icons** - Link icon hashes to abilities/items

## GOM Binary Property Format (Abilities)

The GOM payload contains properties in `[u16 propId] [f32 value]` format.
Property IDs are in the 0x04xx range.

### Confirmed Property IDs (100% Verified)

| ID | Name | Description | Verification |
|----|------|-------------|--------------|
| `0x0401` | **Cooldown** | Ability cooldown in seconds | force_charge=15, force_leap=15, force_push=60, intimidating_roar=60, unload=15, crushing_darkness=15 |
| `0x041b` | **Cast/Channel Time** | Activation time in seconds | snipe=1.5, unload=3.0, lightning_strike=1.5, mortar_volley=3.0 |
| `0x0406` | **Channel Duration** | Duration of channeled abilities | unload=3.0, force_lightning=3.0, mortar_volley=3.0 (matches 0x041b for channels) |
| `0x0403` | **Force Cost (Ranged)** | Force cost for Sorcerer/Sage abilities | crushing_darkness=40, lightning_strike=40, reanimation=30, dark_heal=85 |
| `0x041e` | **Resource Cost** | Energy/Heat cost for Tech; Force cost for melee Force users | shiv=15, rocket_punch=15, rail_shot=15, maul=40, lacerate=20 |
| `0x041a` | **Cast Time (Hard Cast)** | Alternative cast time property | crushing_darkness=2.0 (2s cast) |

### Confirmed Range Properties

| ID | Name | Description | Verification |
|----|------|-------------|--------------|
| `0x041f` | **Melee Range** | Max range for melee abilities (meters) | force_scream=4, smash=3 |
| `0x041d` | **AoE Radius** | Radius for PBAoE abilities (meters) | overload=10 |

Note: Standard 30m ranged abilities do NOT store range explicitly (default assumed).

### Unknown Property IDs (Seen but Purpose Unclear)

| ID | Values Seen | Hypothesis | Evidence |
|----|-------------|------------|----------|
| `0x0402` | 90, 120, 180, 270, 360 | Internal coefficient/scaling | saber_ward=180, recklessness=90, polarity_shift=120 |
| `0x0404` | 3, 20 | Tick count or damage modifier | affliction=3, snipe=20 |
| `0x0442` | 2 | Tick interval or DoT coefficient | affliction=2, crushing_darkness=2 |
| `0x0420` | 1 | Gap closer flag | force_charge=1, force_leap=1 |
| `0x0421` | 1 | Knockback flag | force_push=1 |

### Verified Examples

```
abl.agent.snipe:
  0x041b = 1.50  (cast time: 1.5 seconds ✓)
  0x0404 = 20.00 (damage modifier?)
  (no 0x0401 - Snipe has no cooldown ✓)

abl.agent.shiv:
  0x0401 = 6.00  (cooldown: 6 seconds ✓)
  0x041e = 15.00 (energy cost: 15 ✓)

abl.bounty_hunter.unload:
  0x0401 = 15.00 (cooldown: 15 seconds ✓)
  0x041b = 3.00  (channel time: 3 seconds ✓)
  0x0406 = 3.00  (channel duration: 3 seconds ✓)

abl.sith_warrior.force_charge:
  0x0401 = 15.00 (cooldown: 15 seconds ✓)
  0x0420 = 1.00  (gap closer flag)

abl.sith_warrior.force_push:
  0x0401 = 60.00 (cooldown: 60 seconds ✓)
  0x0421 = 1.00  (knockback flag)

abl.sith_warrior.force_scream:
  0x0401 = 12.00 (cooldown: 12 seconds ✓)
  0x041f = 4.00  (melee range: 4 meters ✓)

abl.sith_inquisitor.crushing_darkness:
  0x0401 = 15.00 (cooldown: 15 seconds ✓)
  0x041a = 2.00  (cast time: 2 seconds ✓)
  0x0403 = 40.00 (force cost: 40 ✓)

abl.sith_inquisitor.lightning_strike:
  0x041b = 1.50  (cast time: 1.5 seconds ✓)
  0x0403 = 40.00 (force cost: 40 ✓)
  (no 0x0401 - no cooldown ✓)

abl.sith_inquisitor.maul:
  0x041e = 40.00 (force cost: 40 - melee ability uses 0x041e ✓)

abl.sith_inquisitor.overload:
  0x041d = 10.00 (AoE radius: 10 meters ✓)
```

### Extraction Method

```python
# Scan for [u16 propId] [f32 value] patterns
for i in range(len(payload) - 6):
    prop_id = struct.unpack_from('<H', payload, i)[0]
    if 0x0400 <= prop_id <= 0x0500:
        value = struct.unpack_from('<f', payload, i + 2)[0]
        # value is likely a game stat
```

## Discipline Tree Ability Mapping

### Tree Structure

SWTOR 7.0+ discipline trees have 13 tier levels:

| Level | Type | Description |
|-------|------|-------------|
| 15 | Core | Auto-granted abilities (2-3 abilities) |
| 23 | Choice | Pick 1 of 3 passives |
| 27 | Choice | Pick 1 of 3 (ability or passive) |
| 35 | Core | Auto-granted ability |
| 39 | Choice | Pick 1 of 3 passives |
| 43 | Choice | Pick 1 of 3 passives |
| 47 | Core | Auto-granted ability |
| 51 | Choice | Pick 1 of 3 (utility passives) |
| 60 | Core | Auto-granted ability |
| 64 | Choice | Pick 1 of 3 (utility passives) |
| 68 | Choice | Pick 1 of 3 (major abilities) |
| 73 | Choice | Pick 1 of 3 (utility passives) |
| 78 | Core | Auto-granted passives |

### FQN Patterns for Discipline Abilities

```
Base discipline abilities:
  abl.{class}.skill.{discipline}.{ability_name}
  Example: abl.jedi_knight.skill.defense.guardian_slash

Discipline passives:
  abl.{class}.skill.{discipline}.mods.passive.{name}
  Example: abl.jedi_knight.skill.defense.mods.passive.critical_defense

Choice tier abilities (tier 2 = lvl 23, tier 3 = lvl 39):
  abl.{class}.skill.{discipline}.mods.tier2.{name}
  abl.{class}.skill.{discipline}.mods.tier3.{name}
  Example: abl.jedi_knight.skill.defense.mods.tier2.defensive_assault

Shared mods (Guardian-wide, not discipline-specific):
  abl.{class}.skill.mods.tier1.{name}
  Example: abl.jedi_knight.skill.mods.tier1.debilitating_slashes

Utility abilities (shared across disciplines):
  abl.{class}.skill.utility.{name}
  Example: abl.jedi_knight.skill.utility.battlefield_command

Special discipline ability:
  abl.{class}.skill.{discipline}.mods.special.{name}
  Example: abl.jedi_knight.skill.defense.mods.special.threatening_focus
```

### Defense Guardian Ability Mapping

#### Tier 15 (Core)
| FQN | Name | Type |
|-----|------|------|
| `abl.jedi_knight.skill.defense.warding_strike` | Warding Strike | Active |
| `abl.jedi_knight.skill.defense.mods.special.threatening_focus` | Threatening Focus | Active |
| `abl.jedi_knight.skill.defense.soresu_form` | Soresu Form | Passive |

#### Tier 23 (Choice - Pick 1)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.defense.mods.tier2.antagnoizing_assault` | Marked Assault | ⚠️ FQN has typo, display name is "Marked Assault" |
| `abl.jedi_knight.skill.defense.mods.tier2.defensive_assault` | Defensive Assault | |
| `abl.jedi_knight.skill.defense.mods.tier2.warding_shield` | Warding Shield | |

#### Tier 27 (Choice - Pick 1)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.cyclone_slash` | Cyclone Slash | Base ability, shared |
| `abl.jedi_knight.skill.mods.tier1.debilitating_slashes` | Debilitating Slashes | Shared mod |
| `abl.jedi_knight.skill.focus.mods.tier1.blade_burst` | Blade Burst | ⚠️ Listed under Focus, may be shared |

#### Tier 35 (Core)
| FQN | Name | Type |
|-----|------|------|
| `abl.jedi_knight.skill.defense.guardian_slash` | Guardian Slash | Active |

#### Tier 39 (Choice - Pick 1)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.defense.mods.tier3.crushing_mark` | Crushing Mark | |
| `abl.jedi_knight.skill.defense.mods.tier3.guardian_focus` | Guardian Focus | |
| `abl.jedi_knight.skill.defense.mods.tier3.critical_slash` | Critical Slash | |
| `abl.jedi_knight.skill.defense.mods.tier3.reinforced_hilt` | Reinforced Hilt | ⚠️ 4th tier3 mod - may be different tier or removed |

#### Tier 43 (Choice - Pick 1)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.defense.mods.passive.marked_focus` | Marked Focus | |
| ??? | Thwart | ⚠️ String ID 925249 exists, FQN not found |
| `abl.jedi_knight.skill.defense.mods.passive.critical_defense` | Critical Defense | |

#### Tier 47 (Core)
| FQN | Name | Type |
|-----|------|------|
| `abl.jedi_knight.skill.defense.hilt_bash` | Hilt Bash | Active (4s stun) |

#### Tier 51 (Choice - Utility)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.utility.unyielding_justice` | Unyielding Justice | |
| `abl.jedi_knight.skill.utility.defiance` | Defiance | |
| `abl.jedi_knight.skill.utility.battlefield_command` | Battlefield Command | |

#### Tier 60 (Core)
| FQN | Name | Type |
|-----|------|------|
| `abl.jedi_knight.skill.defense.warding_call` | Warding Call | Active (40% DR) |

#### Tier 64 (Choice - Utility)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.utility.stalwart_defense` | Stalwart Defense | |
| `abl.jedi_knight.skill.utility.war_master` | War Master | |
| ??? | Second Wind | ⚠️ Not found in database |

#### Tier 68 (Choice - Major)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.awe` | Awe | Base ability |
| ??? | Saber Reflect | ⚠️ **NOT IN DATABASE** - May be 7.0+ addition |
| `abl.jedi_knight.blade_blitz` | Blade Blitz | Base ability |

#### Tier 73 (Choice - Utility)
| FQN | Name | Notes |
|-----|------|-------|
| `abl.jedi_knight.skill.utility.purifying_sweep` | Purifying Sweep | |
| `abl.jedi_knight.skill.utility.gather_strength` | Gather Strength | ✅ Fixed via ETL override |
| ??? | Through Peace | ⚠️ Not found - may have different FQN |

#### Tier 78 (Core Passives)
| FQN | Name | Type |
|-----|------|------|
| `abl.jedi_knight.skill.defense.blade_barrier` | Blade Barrier | Passive |
| `abl.jedi_knight.skill.defense.blade_barricade` | Blade Barricade | Passive |
| `abl.jedi_knight.skill.defense.defensive_swings` | Defensive Swings | Passive |

### Known Data Quality Issues

#### 1. FQN Typos (Legacy Names)
| FQN | Typo | Correct Display Name |
|-----|------|---------------------|
| `defense.mods.tier2.antagnoizing_assault` | "antagnoizing" | Marked Assault / Antagonizing Assault |

#### 2. Missing Abilities (Not in GOM Objects)
| Ability | String ID | Notes |
|---------|-----------|-------|
| Saber Reflect | 727866, 1003740 | String exists but no matching object FQN |
| Thwart | 925249 | String exists but no matching object FQN |
| Second Wind | - | Not found in strings or objects |
| Through Peace | - | Not found in strings or objects |

#### 3. Wrong String Associations (FIXED via ETL overrides)
| FQN | Expected | Actual | Status |
|-----|----------|--------|--------|
| `utility.gather_strength` | Movement impair damage buff | "Ardun Kothe regains his strength..." (NPC ability) | ✅ Override added |
| `defense.mods.special.threatening_focus` | Player taunt + damage | "Taunts the target, forcing it to attack the companion..." | ✅ Override added |
| `immortal.mods.special.unrelenting_rage` | Tank Crushing Fist | "Raging Burst deals 15% more damage..." (DPS ability) | ✅ Override added |

#### 4. Cross-Discipline Ability Placement
| FQN | Listed Discipline | Should Be |
|-----|-------------------|-----------|
| `focus.mods.tier1.blade_burst` | Focus | Shared (all Guardian specs) |

### Hypotheses

1. **Saber Reflect Missing**: May have been added in 7.0 with a completely new FQN pattern not matching `abl.{class}.*`
2. **String Mismatches**: Game reuses string IDs across patches, old FQNs point to updated strings
3. **Thwart/Second Wind/Through Peace**: May be under different FQN patterns like `abl.{class}.skill.guardian.*` (combat style level, not class level)

## Quest GOM Payload Structure

Quest (`qst.*`) payloads contain rich quest definition data.

### Payload Header

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0-1 | 2 | Unknown | `63 20` (99, 32) |
| 2-11 | 10 | Header Data | `cf 40 00 00 00 2a fe 1a 38 06` |
| 12 | 1 | FQN Length | Length of FQN string |
| 13+ | N | FQN | ASCII quest FQN |

### Embedded Data Types

Quest payloads contain multiple data types:

#### 1. Object GUIDs (8 bytes, starts with E0 or C7)
```
E000C766F2DA1A7C  → Self-reference (quest GUID)
E00041F86A600B22  → Related object reference
```

#### 2. String References
| Pattern | Purpose |
|---------|---------|
| `str.qst` | Quest string table (name, description) |
| `str.abl` | Ability string references |

#### 3. NPC/Location References
| Pattern | Example |
|---------|---------|
| `spn.*` | `spn.location.ord_mantell.trainer.class.trainer_trooper_garnik` |
| `npc.*` | `npc.location.ord_mantell.trainer.class.trainer_trooper_garnik` |

#### 4. Quest Mechanics

| Pattern | Purpose | Example |
|---------|---------|---------|
| `_bN_sN_tN` | Branch/Step/Task IDs | `_b1_s2_t1` = Branch 1, Step 2, Task 1 |
| `hook_*` | Event hooks | `hook_talk_to_trooper_trainer` |
| `track_*` | Objective tracking | `track_talk_to_trooper_trainer` |
| `jrn_*` | Journal entries | `jrn_start_talk_to_trooper_trainer` |
| `hyd_complete` | Completion state | Hydra system completion flag |
| `Always` | Condition (always true) | Used in conditional branches |

#### 5. Codex Unlocks
| Pattern | Purpose |
|---------|---------|
| `codexGrantEntry` | Action to grant codex entry |
| `cdx.*` | `cdx.location.ord_mantell.fort_garnik` |

### Example Quest Data: bestofthebest (Trooper Class Quest)

```
FQN: qst.location.ord_mantell.class.trooper.bestofthebest
GUID: E000C766F2DA1A7C

NPCs:
  - spn.location.ord_mantell.trainer.class.trainer_trooper_garnik
  - npc.location.ord_mantell.trainer.class.trainer_trooper_garnik

Quest Branches:
  - _b1_s1_t1 (Branch 1, Step 1, Task 1)
  - _b1_s2_t1 (Branch 1, Step 2, Task 1)

Hooks:
  - hook_talk_to_trooper_trainer
  - track_talk_to_trooper_trainer

Journal:
  - jrn_start_talk_to_trooper_trainer

Codex:
  - cdx.location.ord_mantell.fort_garnik
```

### Mission Tracker Data Model Implications

For the mission tracker feature, quest GOM data provides:

1. **Quest FQN/GUID** - Unique identifiers for database
2. **NPC References** - Quest givers (`spn.*`, `npc.*`)
3. **Branch Structure** - Quest stages and optional paths
4. **Codex Unlocks** - Related codex entries to track
5. **String References** - Links to quest names/descriptions

## Mission Tracker Data Model (GOM-Only)

The mission tracker can be built entirely from GOM data without CSV imports.

### Data Sources (Kessel v2 - 7.8b Extraction)

| Source | Count | Purpose |
|--------|-------|---------|
| `qst.*` objects | 278 | Quest definitions |
| `mpn.*` objects | 9,852 | Mission points (objectives, waypoints) |
| `str.qst.*` strings | 45,356 | Quest display names/descriptions |
| FQN patterns | - | Chapter/act ordering |

**Total quests view (qst.* + mpn.*)**: 10,130 records

### Quest String ID Mapping

Quest objects embed string IDs in their payloads. The format is:

```
Payload pattern: ce XX XX XX YY YY YY YY "str.qst"
                 ^  ^^^^^^^  ^^^^^^^^^^^
                 |  id2 (3B) id1 (4B LE)
                 marker
```

Example: `ce 02 14 45 00 00 00 58` → `str.qst.88.136261` → "Best of the Best"

- `id2` = 0x021445 = 136261 (3-byte big-endian)
- `id1` = 0x00000058 = 88 (4-byte little-endian)

Lookup: `SELECT text FROM strings WHERE id1 = 88 AND id2 = 136261`

### Quest Extraction from mpn.* FQNs

Quest names are embedded in mission point FQNs:

```
mpn.location.{planet}.class.{class}.{quest_name}.{objective}
mpn.location.{planet}.world.{quest_name}.{objective}
mpn.location.{planet}.bonus.{quest_name}.{objective}
mpn.exp.{expansion}.{chapter}.{quest_name}.{objective}
```

### Quest Distribution by Planet/Class

| Planet | Total | Republic Classes | Empire Classes |
|--------|-------|------------------|----------------|
| Nar Shaddaa | 512 | Mixed | Mixed |
| Corellia | 502 | knight:19, consular:12, trooper:24, smuggler:27 | warrior:15, inquisitor:12, hunter:10 |
| Belsavis | 431 | Mixed | Mixed |
| Tatooine | 395 | Mixed | Mixed |
| Alderaan | 393 | Mixed | Mixed |

### Quest Ordering from FQN Patterns

#### Base Game (Vanilla)
| Pattern | Description | Order |
|---------|-------------|-------|
| `chapter_1` | Act 1 | 1 |
| `chapter_2` | Act 2 | 2 |
| `chapter_3` | Act 3 | 3 |

#### Expansions (KotFE/KotET)
| Pattern | Episodes | Order |
|---------|----------|-------|
| `exp.01` | Rise of the Hutt Cartel | 4 |
| `exp.02` | Shadow of Revan | 5 |
| `exp.seasons.01.ep_01-16` | KotFE/KotET (16 episodes) | 6-21 |
| `exp.03` | Onslaught | 22 |
| `exp.04` | Legacy of the Sith | 23 |

#### Planet Progression (Hardcoded)

**Republic:**
```
1. Ord Mantell (Trooper/Smuggler) / Tython (Knight/Consular)
2. Coruscant
3. Taris
4. Nar Shaddaa
5. Tatooine
6. Alderaan
7. [Act 2]
8. Balmorra (Republic version)
9. Quesh
10. Hoth
11. [Act 3]
12. Belsavis
13. Voss
14. Corellia
15. Ilum
```

**Empire:**
```
1. Hutta (Agent/Hunter) / Korriban (Warrior/Inquisitor)
2. Dromund Kaas
3. Balmorra (Empire version)
4. Nar Shaddaa
5. Tatooine
6. Alderaan
7. [Act 2]
8. Taris (Empire version)
9. Quesh
10. Hoth
11. [Act 3]
12. Belsavis
13. Voss
14. Corellia
15. Ilum
```

### Prerequisite DAG Construction

Prerequisites derived from:
1. **Chapter ordering**: `chapter_2` quests require `chapter_1` completion
2. **Planet ordering**: Coruscant quests require starter planet completion
3. **Same-planet ordering**: Based on FQN alphabetical order or manual curation

### Database Schema (D1)

```sql
-- Missions table (populated from GOM)
CREATE TABLE missions (
  fqn TEXT PRIMARY KEY,          -- mpn-derived or qst.* FQN
  guid TEXT,                      -- From qst.* if available
  name TEXT NOT NULL,             -- From str.qst.* or FQN slug
  description TEXT,               -- From str.qst.*

  mission_type TEXT NOT NULL,     -- class, planetary, flashpoint, etc.
  faction TEXT,                   -- republic, empire, both
  origin_fqn TEXT,                -- Class restriction (FK to origins)

  planet TEXT,                    -- Extracted from FQN
  chapter TEXT,                   -- vanilla, rothc, sor, kotfe, kotet, etc.
  chapter_order INTEGER,          -- Order within chapter

  quest_giver_npc TEXT,           -- spn.*/npc.* reference from payload
  codex_unlocks TEXT              -- JSON array of cdx.* FQNs
);

-- Prerequisites DAG (built from ordering rules)
CREATE TABLE mission_prerequisites (
  mission_fqn TEXT NOT NULL,
  prerequisite_fqn TEXT NOT NULL,
  prereq_type TEXT DEFAULT 'required',
  PRIMARY KEY (mission_fqn, prerequisite_fqn)
);
```

### ETL Pipeline

```
1. Extract mpn.* FQNs → Parse quest names, planets, classes
2. Match str.qst.* strings → Display names/descriptions
3. Parse qst.* payloads → NPC refs, codex unlocks
4. Apply ordering rules → Build prerequisite DAG
5. Load to D1 → missions + mission_prerequisites tables
```

### Not in GOM (Manual/Community Data)

| Data | Status |
|------|--------|
| Map coordinates | Not available |
| Alignment points | May be in GOM payload (not decoded) |
| Exact rewards | May be in GOM payload (not decoded) |
| Video walkthroughs | External content |

## Item GOM Structure

### Item String ID Mapping

Item objects reference strings via packed IDs in payload:

```
Format: str.itm#{packed_id}
Decode: id1 = packed_id & 0xFFFFFFFF
        id2 = (packed_id >> 32) & 0xFFFFFFFF
Lookup: SELECT text FROM strings WHERE id1 = ? AND id2 = ?
```

Example: `str.itm#4261883162918912` → id1=0, id2=992292 → "Advanced Armoring 77"

### Item FQN Patterns

| Pattern | Example | Description |
|---------|---------|-------------|
| `itm.gen.armor.{slot}.{class}.{tier}` | `itm.gen.armor.chest.bounty_hunter.01` | General armor |
| `itm.gen.weapon.{type}.{class}.{tier}` | `itm.gen.weapon.blaster.agent.02` | Weapons |
| `itm.mod.armoring.ilvl_{NNN}.{quality}` | `itm.mod.armoring.ilvl_0134.artifact.rand.att_mast_end` | Armoring mods |
| `itm.mod.mod.ilvl_{NNN}.{quality}` | `itm.mod.mod.ilvl_0102.prototype.att_crit` | Mods |
| `itm.mod.enhancement.ilvl_{NNN}.{quality}` | `itm.mod.enhancement.ilvl_0102.artifact.att_alac_end_pwr` | Enhancements |
| `itm.setbonus.{set}.{role}.{effect}` | `itm.setbonus.sow.general.offensive.damage_increase` | Set bonus gear |
| `itm.mtx.{type}...` | `itm.mtx.mount.speeder.longspur` | Cartel Market items |

### Item Level Encoding

Item level is stored as a u8 in the payload after the visual reference string:

```
Pattern: 05 43 02 02 {ilvl} 01 05 05 02 05
Example iLvl 10:  05 43 02 02 0a 01 05 05 02 05
Example iLvl 134: 05 43 02 02 86 01 05 05 02 05
```

### Item Quality from FQN

| Quality | FQN Segment | Rarity |
|---------|-------------|--------|
| standard | `.standard.` | White |
| premium | `.premium.` | Green |
| prototype | `.prototype.` | Blue |
| artifact | `.artifact.` | Purple |
| legendary | `.legendary.` | Gold |

### Item Stat Types from FQN

| Suffix | Primary | Secondary |
|--------|---------|-----------|
| `att_mast_end` | Mastery | Endurance |
| `att_end_mast` | Endurance | Mastery |
| `att_crit` | Critical |  |
| `att_alac` | Alacrity |  |
| `att_alac_end_pwr` | Alacrity | Endurance + Power |

### Item Counts (7.8b Extraction)

| Category | Count |
|----------|-------|
| `itm.gen.*` | 44,313 |
| `itm.mod.*` | 18,569 |
| `itm.eq.*` | 8,325 |
| `itm.mtx.*` | 7,593 |
| `itm.stronghold.*` | 2,597 |
| `itm.schem.*` | 2,299 |
| `itm.setbonus.*` | 845 |
| **Total** | **94,021** |

## Schematic GOM Structure

### Schematic Object References

Schematics contain embedded item references for materials and output:

```
Pattern: cf e0 00 [6 bytes guid] [quantity byte]
         ^^^^^^^^^^              ^^
         Object ref marker       Single byte quantity (0-99)
```

### Decoded Recipe Example

```
schem.tactical.sow.armstech.tactical_1

CRAFTING (first occurrence):
  80x Processed Isotope Stabilizer
  18x Solid Resource Matrix
  15x Artifact Dallorian Researched Component (intermediate)
  20x Legendary Ember
  OUTPUT: Combat Medic Training (tactical item)

RE RETURNS (second occurrence, smaller quantities):
  4x Processed Isotope Stabilizer
  3x Solid Resource Matrix
  5x Artifact Dallorian Researched Component
  0x Legendary Ember
```

### Pattern: Materials Appear Twice

Materials appear twice in schematic payloads:
1. **First set**: Crafting requirements (high quantities)
2. **Second set**: Reverse Engineering returns (lower quantities, some 0)

### Schematic Counts (7.8b)

| Category | Count | Description |
|----------|-------|-------------|
| `schem.gen.*` | 5,626 | General item schematics |
| `schem.mod.*` | 5,085 | Mod/enhancement schematics |
| `schem.eq.*` | 521 | Equipment schematics |
| `schem.conquests.*` | 54 | Conquest supply schematics |
| `schem.tactical.*` | 11 | Tactical item schematics |
| `schem.set_bonus.*` | 10 | Set bonus gear schematics |
| **Total** | **13,774** | |

### Crafting Material Categories

| FQN Pattern | Count | Description |
|-------------|-------|-------------|
| `itm.mat.craft.sca.*` | 37 | Scavenging materials |
| `itm.mat.craft.bio.*` | 31 | Bioanalysis materials |
| `itm.mat.craft.arc.*` | 83 | Archaeology materials |
| `itm.mat.craft.sli.*` | 27 | Slicing materials |
| `itm.mat.craft.supplement.*` | 40 | Vendor supplements (flux, etc.) |
| `itm.mat.craft.endgame.*` | - | Endgame crafting materials |

### Schematic Decoding Code

```python
def decode_schematic_materials(payload):
    """Extract materials and quantities from schematic payload."""
    materials = []
    i = 0
    while i < len(payload) - 10:
        if payload[i:i+3] == b'\xcf\xe0\x00':
            ref_guid = payload[i+3:i+9].hex()
            qty = payload[i+9] if payload[i+9] < 100 else 0
            obj_fqn = find_object_by_guid(ref_guid)
            if obj_fqn and 'itm.mat.' in obj_fqn:
                materials.append((obj_fqn, qty))
            i += 10
        else:
            i += 1
    # First half is crafting reqs, second half is RE returns
    mid = len(materials) // 2
    return {
        'craft': materials[:mid],
        're_returns': materials[mid:]
    }
```

## Hub-Spoke Data Model

### Current State (7.8b v3 Extraction)

| Entity | Count | Contains Refs To | Status |
|--------|-------|------------------|--------|
| Items (`itm.*`) | 94,021 | - | ✅ Extracted |
| Schematics (`schem.*`) | 13,774 | Materials, Output Items | ✅ Decoded |
| NPCs (`npc.*`) | 34,583 | - | ✅ Extracted |
| Trainer Packages (`pkg.*`) | 6 | Schematic lists | ✅ NEW |
| Loot Tables (`loot.*`) | 89 | Junk items, lockboxes | ✅ NEW |
| NPC Vendors | 1,272 | (inventory elsewhere) | ⚠️ No item refs |
| Box Items | 2,227 | (contents elsewhere) | ⚠️ Not decoded |

### New in v3: Trainer Packages

`pkg.profession_trainer.*` objects contain lists of schematics taught by trainers:

```
pkg.profession_trainer.synthweaving_base   → 232 schematic refs
pkg.profession_trainer.armormech_base      → trainer schematics
pkg.profession_trainer.armstech_base       → trainer schematics
pkg.profession_trainer.artifice_base       → trainer schematics
pkg.profession_trainer.biochem_base        → trainer schematics
pkg.profession_trainer.cybertech_base      → trainer schematics
```

**Decoding**: Uses same `cf e0 00 [6-byte guid]` pattern as schematics.

### New in v3: Loot Tables

`loot.*` objects define mob drops and lockbox contents:

```
loot.global.junk.beastamphibian.level_030   → junk drops by mob type/level
loot.global.junk.droidprotocol.level_010    → droid junk
loot.lockbox.artifact_advanced_018          → lockbox definitions
```

**Note**: Lockboxes reference items via string IDs, not embedded refs.

### Still Missing

1. **Vendor Inventories**: NPC vendor payloads don't contain item refs
2. **Quest Rewards**: Quest objects don't embed reward item refs
3. **Box Contents**: Contents determined at runtime, not in GOM

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

5. **Where are 7.0+ discipline tree abilities?**
   - Saber Reflect, Thwart, Second Wind, Through Peace not found
   - May need to search for alternative FQN patterns
   - May require re-extraction from newer game version

## ETL Issues (scripts/etl-abilities.ts)

### String Matching and Override System

The ETL (`scripts/etl-abilities.ts`) matches abilities to names using this pattern:
1. Extract last FQN segment (e.g., `antagnoizing_assault`)
2. Convert to slug: replace underscores with spaces, lowercase
3. Look up in strings table by slug matching
4. Check `FQN_STRING_OVERRIDES` constant first for known problem FQNs

**Implemented Fix**: Added `FQN_STRING_OVERRIDES` constant that handles:
- FQN typos (e.g., "antagnoizing" instead of "antagonizing")
- Wrong string associations (e.g., NPC abilities matched instead of player)
- Legacy FQN names that don't match current display names

### Current FQN Overrides (RESOLVED)

```typescript
// From scripts/etl-abilities.ts - FQN_STRING_OVERRIDES constant
const FQN_STRING_OVERRIDES = {
  // FQN typos
  'abl.jedi_knight.skill.defense.mods.tier2.antagnoizing_assault': 'Marked Assault',
  'abl.sith_warrior.skill.immortal.mods.tier2.targeted_assault': 'Targeted Assault',

  // FQN slug doesn't match any string
  'abl.jedi_knight.skill.defense.mods.tier3.reinforced_hilt': 'Reinforced Hilt',
  'abl.sith_warrior.skill.immortal.mods.tier3.reinforced_hilt': 'Reinforced Hilt',

  // Wrong string associations (NPC/companion abilities instead of player)
  'abl.jedi_knight.skill.utility.gather_strength': 'Gather Strength',
  'abl.sith_warrior.skill.utility.pooled_hatred': 'Pooled Hatred',
  'abl.jedi_knight.skill.defense.mods.special.threatening_focus': 'Threatening Focus',
  'abl.sith_warrior.skill.immortal.mods.special.unrelenting_rage': 'Crushing Fist',
};
```

### Previously Missing Abilities (NOW FIXED)

| FQN | GUID | Status |
|-----|------|--------|
| `abl.jedi_knight.skill.defense.mods.tier2.antagnoizing_assault` | E000CF95761D947D | ✅ Fixed via override → "Marked Assault" |
| `abl.jedi_knight.skill.defense.mods.tier3.reinforced_hilt` | E00063146980D6EA | ✅ Fixed via override → "Reinforced Hilt" |
| `abl.jedi_knight.skill.utility.gather_strength` | E000AD36332A3FEC | ✅ Fixed via override (was matching NPC string) |
| `abl.jedi_knight.skill.defense.mods.special.threatening_focus` | E0007459DE031380 | ✅ Fixed via override (was matching companion string) |
| `abl.sith_warrior.skill.immortal.mods.special.unrelenting_rage` | E000E258B88BE2C1 | ✅ Fixed via override → "Crushing Fist" |

### Still Missing from Database (True Gaps)

| Ability | Notes |
|---------|-------|
| Saber Reflect | May be 7.0+ addition with different FQN pattern |
| Thwart | Not found in GOM objects |
| Second Wind | Not found in strings or objects |
| Through Peace | Not found in strings or objects |
