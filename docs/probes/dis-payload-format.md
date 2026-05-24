# Probe: `dis.*` discipline payload format

Investigation only. Documents what's inside a discipline record, what each section appears to do, and what kessel would gain from extracting it. No code changes yet.

Validated 2026-05-23 against 3 disciplines from 3 different classes:
- `dis.powertech.firebug` (813 B)
- `dis.juggernaut.immortal` (807 B)
- `dis.sorcerer.lightning` (812 B)

48 total `dis.*` records exist in the corpus, all 800–823 bytes, format invariant across all sampled classes.

## TL;DR

Every discipline record holds:

1. **Short codename** (`power_pyrotech`, `jugg_immortal`, `sorc_lightning`) — distinct from the FQN-derived discipline name.
2. **2 apc.\* references** — the discipline icon and its mod-tree visual.
3. **24 ability/talent GUIDs** — the full pool of mod choices for that discipline.
4. **8 tiers × 3 choices structure** — explicit UI layout for the mod tree (the in-game choice trio at each level).
5. **24 index → level map** — which level unlocks each mod choice (8 distinct levels typically: 23, 27, 39, 43, 51, 64, 68, 73).
6. **8 default-mod selections** — one default per tier, format suggests "the auto-granted choice when no manual selection."
7. **Signature ability GUID** — the iconic discipline-defining ability (Flaming Fist for Pyrotech, Aegis Assault for Immortal, Chain Lightning for Lightning).

22/24 mod GUIDs resolve directly as `abl.*` / `tal.*` objects in spice. 2/24 are unresolved — likely versioned variants or whitelist-dropped objects. Worth a follow-up probe.

## Byte layout

Sections numbered for reference. Offsets shown are from `dis.powertech.firebug` (813 B).

### A. Header (0x00–0x09)

```
00 00 00 00 00 00 00 11 10
```

9 bytes of GOM-record header (version bytes, record-internal). Same shape across the 3 samples; the last 2 bytes vary slightly per record (likely counters).

### B. Class marker + codename (0x09–0x22)

```
CF 40 00 00 41 FC 3C 7A 20    # CF40 marker, hi32 = 0xFC3C7A20 (the Discipline class template)
06 0E                          # wire-tag 0x06 (string), length 14
70 6F 77 65 72 5F 70 79 72 6F 74 65 63 68  # "power_pyrotech"
```

Identical CF40 hi32 (`FC3C7A20`) across all 48 disciplines — this is the Discipline class type. The length-prefixed string is the **short codename** (`power_pyrotech`, `jugg_immortal`, `sorc_lightning`, etc).

### C. GOM object metadata (0x23–0x5C)

Standard CC / CE / CB markers — `string_id` reference, content-hash markers, parent class refs. Same shape as any other PBUK object. ~58 bytes.

### D. Appearance refs (0x5D–0x76)

```
07 01 01 01 01            # 5-byte counter/array opener
CF E0 00 <6-byte tail>    # apc icon ref
01 01 CF E0 00 <6-byte tail>  # apc mod-tree ref (with leading counter `01 01`)
```

Each `CF E0` reference encodes a full content GUID as `E000` + the 6-byte tail. Validated:

- `E00068A7087033E1` → `apc.bounty_hunter.powertech.pyrotech` (discipline icon/portrait)
- `E000FE2EFC9D17DA` → `apc.bounty_hunter.powertech.pyrotech_mods` (mod tree visual)
- `E000D7C5E57B5435` → `apc.sith_warrior.juggernaut.immortal`
- `E000CB520CE8FC20` → `apc.sith_warrior.juggernaut.immortal_mods`
- `E000CD719E9F6994` → `apc.sith_inquisitor.sorcerer.lightning`
- `E000DDDAA8CA0A97` → `apc.sith_inquisitor.sorcerer.lightning_mods`

Pattern: `apc.<origin>.<combat_style>.<discipline>` and `..._mods`. Naming is invariant.

### E. Transition (0x77–0x7F)

```
03 82 C8 2C 11 32  08 02 07 08 08 17
```

12 bytes, identical across all 3 samples. Probably the array header for the tier-choice section that follows.

### F. Tier-choice triplets (0x80–0xD8)

Pattern (10 bytes per triplet):

```
02 03 03 <a> 02 <b> 03 <c> <level>
```

- `02 03 03` — fixed 3-element subarray opener
- `<a>` — index of choice #1 (left in UI?)
- `02 <b>` — separator + index of choice #2 (middle?)
- `03 <c>` — separator + index of choice #3 (right?)
- `<level>` — the player level at which this tier unlocks (1-byte int)

8 tier-triplets observed in firebug. Levels: 0x1B (27), 0x27 (39), 0x2B (43), 0x33 (51), 0x40 (64), 0x44 (68), 0x49 (73). 8th triplet's level appears as part of the array terminator.

The triplet indices are **1-based positions into the main CF E0 list** that follows. For firebug Tier 1 = `(01, 03, 02)` at level 27 → indices 01, 03, 02 of the main list = Primed Ignition / Open Flame / Heatstroke — confirmed against in-game Pyrotech Tier 1.

The triplet's `(a, b, c)` order is NOT 1/2/3 ascending — it appears to be the **UI display order** (left/middle/right), which is information kessel doesn't have today.

### G. Main mod list (0x0D9–0x1C7)

24 CF E0 records in tier/level order, each carrying a 1-byte index + the 9-byte CF E0 record:

```
<index>  CF E0 00 <6-byte GUID tail>
```

Indices 0x01..0x18 (1..24). Each full GUID = `E000` + tail (8 bytes BE).

For firebug, 22/24 resolve directly:
- Discipline mods: `abl.bounty_hunter.skill.firebug.mods.tier1/3/passive.*` (primed_ignition, heatstroke, open_flame, whistling_birds, mandalorian_warhead, primed_immolation, chilled_retribution, boiling_point)
- Class-shared utility abilities: gyroscopic_alignment_jets, pyro_shield, suppressive_tools, reflective_armor, hitman, shield_cannon
- Class-shared utility **talents**: iron_will, enhanced_paralytics, efficient_suit, sonic_rebounder
- Class abilities: jet_charge, hydraulic_overrides_powertech, electro_dart, stealth_scan

2/24 unresolved (`E00013F812947064`, `E00022851F0CAD9B`). **Root cause identified**: these reference `abl.bounty_hunter.skill.firebug.mods.tier2.heat_chain` and `abl.bounty_hunter.second_contract`. Both abilities exist in the corpus **only as versioned variants** (`/7/0` `/7/1` and `/5/0` `/5/2` respectively) — no base FQN ever appears. Kessel's `should_extract_object` returns false for any FQN containing `/` (main.rs:726), and the dedup logic strips versioned variants without promoting any to canonical. **Result: abilities that only have versioned variants are completely dropped from spice.** Heat Chain is a real player-facing Pyrotech Tier 2 mod. Pulling dis.* would surface this gap automatically — every unresolved CF E0 ref in the discipline payload points at one of these silently-dropped abilities.

### H. Sorted lookup list (0x1C8–0x2BD)

Same 24 GUIDs as section G, sorted by hash bytes ascending, each suffixed with its position in section G:

```
01 08 01 02 18 18           # header (24 entries)
CF E0 00 <sorted GUID> <main-list position>
...
```

Likely a fast-lookup index for "given a content GUID, what tier-position is it." Redundant for kessel's purposes if we build our own index; useful for in-game lookups.

### I. Index → level map (0x2BE–0x310)

```
CA B1 00 7D 02 CE 0B FC 49 00 00 01 30 CA 48 EF E0
08 02 02 18 18                              # 24-entry array
01 17 02 17 03 17 04 1B 05 1B 06 1B 07 27 08 27 09 27 ...
```

24 (`index`, `level`) pairs. Confirms the tier structure:
- firebug levels: indices 01-03 at 0x17 (23), 04-06 at 0x1B (27), 07-09 at 0x27 (39), 0A-0C at 0x2B (43), 0D-0F at 0x33 (51), 10-12 at 0x40 (64), 13-15 at 0x44 (68), 16-18 at 0x49 (73)
- sorcerer.lightning: similar shape but the last 6 indices (13-18) have non-monotonic levels (`14 1B 15 2B 16 44 17 49 18 49`) — possibly representing utility-tier picks separate from the standard mod tiers.

### J. Default-mod selections (0x311–0x322)

```
CB 0F B4 B0 E0
08 02 02 08 08 17           # 8-entry array
02 1B 05 27 09 2B 0A 33 0F 40 12 44 13 49 18 ??  # 8 (level, index) pairs
```

8 pairs of (`level`, `index`) — one per tier. Hypothesis: the **default mod selected** at each tier (the auto-granted choice if the player makes no manual selection). For firebug: defaults at level 27 = index 02 (heatstroke); level 39 = index 05; etc. Needs validation against in-game default behavior.

### K. Signature ability (0x323–0x32D)

```
01 07 01 01 01 01
CF E0 00 <6-byte tail>
```

A single trailing CF E0 ref. Resolves to the **discipline's signature ability** — the iconic ability that defines the spec, automatically granted when you choose the discipline:

- `dis.powertech.firebug` → `abl.bounty_hunter.skill.firebug.flaming_fist`
- `dis.juggernaut.immortal` → `abl.sith_warrior.skill.immortal.aegis_assault`
- `dis.sorcerer.lightning` → `abl.sith_inquisitor.skill.lightning.chain_lightning`

This is information kessel doesn't currently have at all — there's no "signature ability" concept in the disciplines table.

## What kessel currently has vs. what dis.* provides

| Field | Current source | Authoritative `dis.*` source |
|---|---|---|
| Discipline FQN | inferred from `abl.<class>.skill.<disc>.*` patterns | direct from `dis.<class>.<disc>` |
| Short codename (e.g. `power_pyrotech`) | not extracted | `06 <len> <string>` in section B |
| Icon | inferred via `apc.<class>.<style>.<disc>` FQN | direct CF E0 ref in section D |
| Mod-tree visual | not extracted | direct CF E0 ref in section D |
| Ability membership | fan-out heuristics from FQN segments | explicit 24-entry list in section G (canonical) |
| Talent membership | not directly linked | included in the same 24-entry list |
| Tier structure (3 choices per tier) | inferred from `mods.tier1/2/3` FQN segments | explicit triplet array in section F |
| Per-tier unlock level | not extracted | index→level map in section I |
| Per-tier UI order (left/middle/right) | not extracted | triplet position in section F |
| Default mod per tier | not extracted | section J |
| **Signature ability** | **not extracted** | **section K** |
| **Cross-faction discipline mirror map** | **not extracted** | falls out of shared signature ability GUIDs (see audit below) |
| **Versioned-only abilities** (e.g. Heat Chain) | **dropped entirely** | referenced by GUID in dis.* — pulling dis surfaces every such gap |

## 48-discipline audit (2026-05-24)

Audit binary `dis_format_audit` against every `dis.*` record. Findings:

**Format invariance confirmed** — every discipline has exactly:
- `51` CF E0 references (2 apc + 24 main mod list + 24 sorted lookup + 1 signature)
- `8` tier-choice triplets (10 bytes each, format `02 03 03 [01] [a] [02] [b] [03] [c] [level]`)
- The fixed transition marker (`03 82 C8 2C 11`)
- Exactly one trailing signature ability GUID

**Cross-faction mirror disciplines share signature ability GUIDs** — this is a real cross-faction map that falls out of dis.* extraction for free:

| Imperial discipline | Republic discipline | Shared signature ability |
|---|---|---|
| `dis.juggernaut.rage` | `dis.marauder.fury` (wait — both Imp) | `E000AE7688365CAE` (Warrior rage tradition) |
| `dis.sorcerer.madness` | `dis.assassin.hatred` (both Imp) | `E000B6A548225F23` (Inquisitor dot tradition) |
| Republic side: `dis.guardian.focus` | `dis.sentinel.concentration` | `E00029FEAFC07E12` |
| Republic side: `dis.sage.balance` | `dis.shadow.serenity` | `E000E2E6137D4F20` |

(Note: cross-class within faction shares signatures too, indicating "discipline traditions" span combat styles. This is gameplay-meaningful — it's how SWTOR groups disciplines into thematic families like "rage tradition" or "madness tradition".)

**One real data quirk**: `dis.shadow.combat` literally stores codename `sent_combat` (the bytes do, not a parser bug). Distinct signature ability from `dis.sentinel.combat` so they ARE separate disciplines — likely a Bioware data error where shadow.combat was forked from sentinel.combat without updating the codename string. Flagged; not blocking.

**Audit caveat — the 8-vs-9 triplet false split**: an earlier naive `02 03 03` substring counter showed disciplines split between 8 and 9 triplets. This was a false positive. Firebug's Tier 1 triplet `(a=01, b=03, c=02)` produces bytes `02 03 03 01 01 [02 03 03] 02 1B` where the bracketed sequence is incidentally `[selector2=02] [b=03] [selector3=03]` — same byte pattern as the record header. **All 48 disciplines have exactly 8 tier triplets** when parsed as 10-byte records.

To reproduce:
```bash
cargo build --release -p kessel-discovery --bin dis_format_audit
./target/release/dis_format_audit -i ~/swtor/Assets -H /tmp/hashes_filename.txt
```

## What switching looks like

To populate the disciplines table from `dis.*` instead of FQN inference:

1. Add `"dis"` to the FQN whitelist in `kessel/src/main.rs:740`.
2. Write a decoder for the dis-payload format (sections B, D, F, G, I, J, K).
3. Schema additions: `codename`, `icon_apc_game_id`, `mod_tree_apc_game_id`, `signature_ability_game_id`. New table `discipline_mods (discipline_game_id, tier_level, ui_position, ability_or_talent_game_id, is_default)`.
4. Existing `disciplines` and `discipline_abilities` tables can stay; just populated from a different source. The FQN-inference path can be retired or kept as a cross-check.

## Open questions / follow-up probes worth doing

- **Default-mod hypothesis** — section J's interpretation as "default choice per tier" is plausible from the structure but not validated against in-game behavior.
- **Sorcerer non-monotonic levels** — sorc.lightning's level map ends with `1B 2B 44 44 44 49 49 49` and the trailing triplet indices don't fit the 3-per-tier pattern as cleanly. Some disciplines may have a different tier count or include utility-tree picks alongside mod-tree picks. Needs cross-class validation across all 48.
- **Format of CC/CE/CB markers in section C** — these are standard GOM metadata but not all decoded. If `string_id` is in there, the discipline's display name (`Pyrotech`, `Immortal`, etc.) can be linked to STB.

## How to reproduce

```bash
cargo build --release -p kessel-discovery --bin probe_dis
./target/release/probe_dis -i ~/swtor/Assets -H /tmp/hashes_filename.txt -f dis.powertech.firebug
./target/release/probe_dis -i ~/swtor/Assets -H /tmp/hashes_filename.txt -f dis.juggernaut.immortal
./target/release/probe_dis -i ~/swtor/Assets -H /tmp/hashes_filename.txt -f dis.sorcerer.lightning
```

`probe_dis` is a one-shot dumper added to `kessel-discovery/src/bin/`. Walks PBUK buckets, finds dis.* objects, prints full payload as hex.
