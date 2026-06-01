# Corpus Atlas

A map of every category of data in the SWTOR `.tor` archives, with one-line labels for what each is. **Does not decode contents** — only locates and names. Use this as the reference for "where does this kind of thing live."

Live-archive numbers captured 2026-05-23 against `~/swtor/Assets` (game build `apesVersion=2961.1.1`, `dbVersion=20260403050009`, code changelist `1076125`).

## 1. Top-level layout of the archives

885,845 distinct file paths across 101 `.tor` archives.

| Top-level dir | Files | Contents |
|---|---|---|
| `art/` | 489,982 | 3D models, textures, materials, particle/FX specs |
| `anim/` | 130,858 | Animations, skeleton specs, animation cue metadata |
| `gfx/` | 94,199 | UI textures (icons, portraits, mtxstore), Scaleform UIs |
| `world/` | 65,070 | Zone definitions, area data, mapnotes, minimaps |
| `gamedata/` | 20,587 | EPP appearance specs, surveys, manifests |
| `systemgenerated/` | 19,855 | Master schemas, scripts, bucket index, prototypes |
| `server/` | 17,655 | Server-side specs (spawn defs, FaceFX, tables) |
| `en-us/` | 14,924 | English string tables (STB), per-line FX (FXE) |
| `fr-fr/` `de-de/` | 14,497 each | Non-English locale data (filtered out of scope) |
| `bnk2/` | 3,479 | Wwise audio bank metadata |
| `alien/` `guixml/` `engine/` `bel_arch_*` | ~200 | One-off art/UI/engine assets |
| `version.txt`, `metadata.bin`, `groupmanifest.bin`, `global.dep`, `ft.sig` | 5 | Top-level singleton meta files |

## 2. PBUK bucket layer — 608,131 in-game objects

The `systemgenerated/buckets/*.bkt` files hold `608,131` GOM objects across `451` distinct FQN prefixes. Kessel today extracts `316,129` (about 52%) — the prefix whitelist in `kessel/src/main.rs:740` accepts 19 prefixes and drops the rest.

### 2a. Per-instance dotted-FQN prefixes (kessel pulls a subset)

Each row is "this prefix is a `<kind>` of object". Source labels come from the FQN itself, the kind hints in payload strings, and existing kessel handling.

| Prefix | Count | Kind | Extracted? |
|---|---:|---|---|
| `abl` | 141,522 | Abilities (player + NPC, ability effect FQNs in payload) | yes |
| `itm` | 129,859 | Items (gear, schematics, materials, mtx) | yes |
| `spn` | 80,307 | Spawn instances (refs npc.* in payload) | yes |
| `npc` | 53,127 | NPCs (refs cnv.* in payload) | yes |
| `hyd` | 34,264 | "Hydra" event/encounter scaffolding (refs encounter `senc.location...`) | **dropped** |
| `plc` | 21,860 | Placeables (interactables, e.g. ship_holoterminal, refs `art/static/area`) | yes |
| `cnd` | 20,065 | **Conditions** (e.g. `cnd.itm.has_item.lots.armor...` — boolean predicates) | **dropped** |
| `npp` | 20,049 | NPC packages (e.g. `npp.location.makeb.mercenaries.infantry_bms_elite`) | **dropped** |
| `epp` | 20,028 | Effect/appearance prototypes (PBUK-side; see also `.epp` files in §5) | partial (player-class only) |
| `schem` | 16,642 | Schematics (crafting recipes) | yes |
| `dyn` | 13,453 | Dynamic placeables (e.g. boss-fight combat objects — railguns, panels) | **dropped** |
| `mpn` | 12,386 | Mission phases ("QuestCircle" marker in payload) | yes |
| `enc` | 11,828 | Encounters | yes |
| `stg` | 7,796 | Stages (cutscene/mission staging; refs conversation postures) | **dropped** |
| `ach` | 7,516 | Achievements (refs ability and codex FQNs) | yes |
| `apn` | 4,723 | Animation packages (e.g. `apn.npc.qtr.1x1.raid...assassin_droid`) | **dropped** |
| `cdx` | 3,946 | Codex entries | yes |
| `apc` | 1,564 | Appearance components (companions, creatures — `apc.companion.class.mtx.creature...`) | yes |
| `qst` | 1,515 | Quests | yes |
| `tal` | 1,210 | Talents | yes |
| `dynamic` | 1,124 | Dynamic garmenthue (cosmetic hue config — `dynamic.garmenthue.garmenthue_deck_officer_primary`) | **dropped** |
| `cos` | 496 | Cost / cosmetics (e.g. `cos.location.taris_imperial...ambient01.strong_r_01` — ambient NPC costume) | **dropped** |
| `pth` | 381 | Paths (NPC pathing — `pth.location.balmorra_republic...foot_trail_interrogation_droid_random`) | **dropped** |
| `world` | 363 | World/area data (mapdata) | **dropped** |
| `tax` | 301 | Taxi routes (`tax.meksha.rep_flight_pad` → refs paths) | **dropped** |
| `lky` | 217 | Movie/cinematic anchors (`lky.movie.kotfe`, `str.lky` strings) | **dropped** |
| `pcs` | 211 | **Player character species presets** (e.g. `pcs.jedi_knight.female.rattataki_legacy`) | **dropped** |
| `nco` | 184 | NPC companions, original era (e.g. `nco.companions_original.warrior.broonmark`) | **dropped** |
| `mrp` | 163 | Mounts / rewards / packages (`mrp.galactic_seasons.season_5.mouse_droid`) | **dropped** |
| `ipp` | 116 | Item paint/pattern prototypes (`ipp.custom.sow.progresson.ge_a13_purple_holo.legs`) | **dropped** |
| `svy` | 77 | Surveys (PBUK side; corresponds to `.svy` XML files) | **dropped** |
| `dis` | 48 | Disciplines (e.g. `dis.powertech.firebug`) | **dropped** |
| `emt` | 45 | Environment materials (PBUK side; corresponds to `.emt` XML files) | **dropped** |
| `cam` | 42 | Cameras (`cam.auto.8-0.medium_wide` — camera presets) | **dropped** |
| `apt` | 16 | Apartment / stronghold layouts (`apt.flagship.imperial_capital_ship`) | **dropped** |
| `class` | 16 | Player class definitions (`class.pc.advanced.sorcerer`) | yes |
| `pkg` | 6 | Profession trainer packages (`pkg.profession_trainer.synthweaving_base`) | yes |

### 2b. PBUK FQN extractor byte-drop bug

The PBUK FQN extractor drops byte 0 on roughly 10 prototype-style names, producing garbage prefixes:

- `cation` (41) → `Location` (the `L` was dropped, joined to a quest-payload header)
- `ami` (19) → likely `Family` or similar
- `n` (30), `p` (29), `st` (27), `m` (11), `r` (18), `c` (5), `d` (14), `x` (4) → all single/short-letter bug-prefixes
- `rdd` (18) → likely `Brdd` or similar (a real prototype)
- `ent` (3), `eration` (9), `liance` (6), `mpanion` (6), `aceables` (2), `dex` (2), `neric` (2), `ashpoint` (5), `th_warrior` (2), `unty_hunter` (2), `urb` (2) → `Event…`, `Federation…`, `Alliance…`, `Companion…`, `Placeables…`, `Codex…`, `Generic…`, `Flashpoint…`, `sith_warrior…`, `bounty_hunter…`, `Suburb…`

Fix queued as "tighten the FQN extractor" side-quest; tracked in `legion recall 019df507`.

### 2c. Singleton master tables (370 of them)

Each "singleton" is a single PBUK object with a zero-dot PascalCase/camelCase FQN that holds an entire game-config table. All 370 are dropped by the FQN-prefix-whitelist filter because the prefix is the whole name (no dot to split on).

| Prototype | Payload size | What it holds |
|---|---:|---|
| `colCollectionItemsPrototype` | 2,290,795 B | Cosmetic collection items master (mtx companions, mounts, decorations) |
| `chrPaidPermissionDefsTablePrototype` | 404,813 B | Cartel-market unlock catalog |
| `achCategoriesTable_Prototype` | 307,924 B | Achievement category tree (refs cdx + ach) |
| `cbtArmorPerLevel` | 279,010 B | Armor scaling curve per level |
| `cbtShieldPerLevel` | 240,935 B | Shield scaling curve per level |
| `colCollectionCategoriesPrototype` | 224,659 B | Collection category tree |
| `cnqConquestInfoPrototype` | 141,169 B | Conquest event definitions |
| `chrCompanionInfo_Prototype` | 86,664 B | Companion info table |
| `cdxCategoryTotalsPrototype` | 86,524 B | Codex category totals |
| `cdxCompletionBonusPrototype` | 83 B | Codex completion bonus |
| `cbrCommandInfo_Prototype` | 34,102 B | Command rank info |
| `colCollectionSourcesPrototype` | 39,693 B | Collection source mappings |
| `cdxBitToFQNPrototype` | 46,926 B | Codex bit → FQN map |
| `chrClassListingPrototype` | 38,250 B | Class listing master |
| `achRewardsTable_Prototype` | 37,013 B | Achievement rewards |
| `chrBackgroundTablePrototype` | 25,004 B | Character background pickers |
| `chrPlayerTitlesTablePrototype` | 22,561 B | Player titles |
| `cnqAchGroupPrototype` | 22,562 B | Conquest achievement groups |
| `ablVanityPetsPrototype` | 23,800 B | Vanity pets master |
| `ablPackagePrototype` | 7,356 B | Ability package master |
| `tagTablePrototype` | 449,917 B | **Tag dictionary — ~7,083 `tag.*` FQNs** |
| `ccsAppearanceTablePrototype` | 7,007 B | Character customization appearance |
| `chrCurrencyTablePrototype` | 6,130 B | Currency definitions |
| `conEquipmentSlotDataPrototype` | 6,532 B | Equipment slot rules |
| `ahItemCategoriesPrototype` | 1,784 B | Auction-house item categories |
| `ahItemSlotCategoriesPrototype` | 272 B | Auction-house item slot categories |
| `ahItemSubCategoriesPrototype` | 1,939 B | Auction-house item subcategories |
| `cnvBaseFxaListPrototype` | 848 B | Conversation FaceFX file list |
| `cnvcameratable` | 7,059 B | Conversation camera presets |
| `cnvReactionsDataPrototype` | 2,515 B | Conversation reactions |
| `cnvRewardTable_Prototype` | 176 B | Conversation reward refs |
| ... | ... | (340 more singletons; see `catalog_singletons` output) |

To regenerate the full singleton list:

```bash
cargo build --release -p kessel-discovery --bin catalog_singletons
./target/release/catalog_singletons -i ~/swtor/Assets -H /tmp/hashes_filename.txt > /tmp/singletons.tsv
```

## 3. `systemgenerated/` — schema, scripts, routing

| File | Magic | Size | What it is |
|---|---|---:|---|
| `client.gom` | `DBLB` | 921 KB | Master GOM schema (748 enums + 10,006 properties + 2,220 classes). Embedded in kessel as `gom_enums.json` / `gom_classes.json` / `gom_properties.json`. |
| `prototypes.info` | `PINF` | 7.2 MB | 723,690 records (10 bytes each: u64 BE id + u8 flag + u8 unknown). **Prior interpretation as "routing key" needs revisiting** — flag bytes 0x00–0xFF all appear evenly (~2,800 each), not as a small enum. Format not fully understood. |
| `buckets.info` | `PBCK` | 7.8 KB | Directory listing of the 997 `.bkt` bucket files (`0.bkt`, `1.bkt`, ...). |
| `scriptdef.list` | `SDEF` | 38 KB | Script definition records (`<u64 id><CF prefix><hash>` per record). |
| `compilednative/<numeric_id>` | `SCPT` | varies | **HeroScript** (HeroEngine), compiled to native x86-64 — engine/systems logic (combat clips, skill trees, GSF physics, UI). **NOT story-content scripts.** Verified 2026-06-01 against the 1,196 `.scpt` bodies in spice: only 8 carry any FQN ref and **0 reference quest flags** (`counter_`/`qm_`/`go_`/`track_`); the bytes are x86 prologues (`push rsi/rdi/rbx` …) + a "Missing return." compiler string. Quest/story progression logic is **not** in here. Kessel has `kessel::scpt` (decrypt only, from #127); decoding the native code would yield engine internals, not content. |
| `buckets/<n>.bkt` | `PBUK` | varies | 997 individual bucket payloads. Each holds many GOM objects. Already consumed by kessel via `pbuk::parse`. |
| `prototypes/<numeric_id>.node` | `PROT` (per agent 019e4d74) | varies | 17,514 prototype files. Per the file-format catalog, contains a mix of cnv + creature + stage + player-ability prototypes. **Caveat**: a sample hash failed to resolve via `dump_epp` — exact present-vs-absent count needs re-verification per .node. |

> **No story-content scripts in the archive (verified 2026-06-01).** Quest/story *content* is declarative — names, objective/journal/description text, progression flags (all in the strings table + quest payloads, extracted) — and is fully captured. The procedural *logic* (objective counts, conversation flow, cross-quest prerequisites, completion conditions, planet order) lives in **HeroScript** (the compiled `.scpt` native engine code above — no quest-flag symbols) and **hydra** (`hyd.*` spawn/POI/encounter event scripting). Neither is a readable "story content script": HeroScript is compiled native x86-64; hydra is spawn/event scaffolding. So those runtime behaviors are not extractable as data without decompiling engine code — they are the runtime-residual ceiling, not a content layer we are missing.

## 4. Singleton meta files (top-level)

| File | Size | Format | What it is |
|---|---:|---|---|
| `version.txt` | 239 B | INI | **Game build version**: `apesVersion=2961.1.1`, `codeChangelist=1076125`, `defsChangelist=1076125`. Not currently stored in spice's `meta` table. |
| `metadata.bin` | 43 KB | binary | 2,690 archive-entry meta records (per prior survey). |
| `groupmanifest.bin` | 2 KB | binary | Lists all `.tor` archive files by group name. |
| `global.dep` | 7.9 MB | binary | Opaque dependency graph (for installer/patcher; not gameplay data). |
| `ft.sig` | 194 B | binary | Cryptographic signature (FaceFX?). |
| `gamedata/str/stb.manifest` | 14 KB | ASCII XML | `<manifest><file val="str.abl.player.grant_codex_xp"/>...` — full listing of every STB string table. |
| `art/dynamic/testrules.rul` | 13 KB | ASCII XML | `<?xml version="1.0"?><Rules>` — equipment-slot tagging rules. |
| `art/lodschemas3.lod` | 2.5 KB | ASCII XML | `<LODSchemaGroup>` — LOD thresholds. |

## 5. Ignored file extensions — first-byte identification

### Containers kessel doesn't open

| Ext | Count | Magic / first bytes | What it is |
|---|---:|---|---|
| `.epp` | 20,527 | `FF FE 3C 00 41 00 70 00 70 00 65 00 61 00 72 00 61 00 6E 00 63 00 65 00` | UTF-16-LE XML `<Appearance fqn="...">`. Per-ability/talent **Appearance Spec** target (e.g. `epp.sith_warrior.massacre.cast_instant`). VFX/SFX/animation triggers. |
| `.fxspec` | 22,654 | `<.n.o.d.e.W.C.l.a.s.s.e.s.>` (UTF-16) | UTF-16-LE XML `<nodeWClasses><classes>`. FX node graphs (FX-to-ability bindings). |
| `.fxe` | 13,124 | `FACE B8 06 00 00 ... Bioware Edmonton` | FACE binary (FaceFX format). Per-line localized facial animation. |
| `.fxa` | 22 | `FACE B8 06 00 00 ...` | Same FACE format. Per-species base facial-animation templates. |
| `.amx` | 15,933 | `41 4D 58 20 ... cb_saber_idle_right_adjust_1 ... humanoid\\bfbnew` | Plaintext `AMX ` magic. Animation cue metadata. NOT a script format. |
| `.svy` | 59 | `FF FE 3C 00 53 00 75 00 72 00 76 00 65 00 79 00 49 00 6E 00 73 00 74 00 61 00 6E 00 63 00 65 00` | UTF-16-LE XML `<SurveyInstance fqn="...">`. In-game player surveys. |
| `.emt` | 50 | `FF FE 3C 00 3F 00 78 00 6D 00 6C 00` | UTF-16-LE XML `<?xml ...>`. Environment-material shader specs. |
| `.tbl` | 4 | (server-only; not in client archives) | DataTable XML per file-format catalog. `chrspec.tbl` = class/spec defs, `fxhuecolors.tbl`, etc. |
| `.dat` | 19,412 | `18 00 00 00 ROOM_DAT_BINARY_FORMAT_` | World room geometry binary. |
| `.xml` | 877 | `<Palette><Brightness>...` | ASCII XML, mostly garmenthue palette specs. |
| `.not` | 272 | `<v><k>mapNotes</k>...` | Map notes XML (zone-level annotations). |
| `.dyc` | 398 | `Version=2..[SETTINGS]..Skeleton=bfnnew_skeleton` | INI-style animation skeleton spec. |
| `.mag` | 325 | `! Mag Specification for ...` | Plaintext magnetic / pathing spec. |
| `.rul` | 1 | `<?xml version="1.0"?><Rules>` | ASCII XML rule set (only `testrules.rul`). |
| `.ini` | 1 | (input keybindings) | Dev keybinding config. |
| `.bkt` | 997 | `PBUK` | Individual bucket file (kessel consumes these via PBUK parser; the file itself is in scope). |
| `.node` | 17,514 | (per catalog: `PROT`, but a sample hash failed to resolve via `dump_epp`) | Prototype files. Mix of cnv + creature + stage + player-ability prototypes. **`cnv.*` .node = conversation CINEMATICS** (camera marks, animation, music cues, actor movement, stage refs) — verified 2026-06-01: a 131 KB cnv NODE carries **no dialogue text** (not ASCII, not UTF-16) and **no narrative next-conversation refs** (the `cnv→cnv` refs present are alien-VO variants). **NOT story content / not dialogue scripts.** The quest↔conversation *link* used downstream (`conversation_quest_refs`) comes from CF-GUID refs, not from decoding the NODE. |

### Files in hash dictionary but NOT in client archives (server-only)

| Ext | Count | Likely contents (path-based) |
|---|---:|---|
| `.spn_c` | 17,427 | Server spawn definitions (`/resources/server/spn/.../<name>.spn_c`) |
| `.spn_crf` | 149 | Server spawn CRF (per CRF refac pattern) |
| `.spn_lst` | 51 | Server spawn list / patrol routes |
| `.abl` | 2 | Server ability definitions (`/resources/server/abl/player/quick_travel_instance.abl`) |
| `.spt` | 106 | SpeedTree foliage |

### Pure art / audio (out of game-data scope)

| Ext | Count |
|---|---:|
| `.dds` | 314,038 |
| `.gr2` | 135,541 |
| `.jba` | 95,555 |
| `.tex` | 87,560 |
| `.prt` | 37,904 |
| `.mat` | 27,616 |
| `.mph` | 19,420 |
| `.acb` | 13,022 |
| `.clo` | 2,571 |
| `.wem` | 2,515 |
| `.bnk` | 865 |
| `.gfx` | 556 |
| `.swf` | 5 |

## 6. Localized string tables (`.stb`)

| Locale | File count | Status |
|---|---:|---|
| `en-us/` | 14,924 | Extracted by kessel |
| `fr-fr/` | 14,497 | Out of scope (per Sean: filter non-English) |
| `de-de/` | 14,497 | Out of scope |

kessel selects STB files via `stb::should_extract_stb` (root-level kind tables `abl/tal/itm/npc/qst/cdx/ach/schem`, the two `gui` category tables, and — as of #281 — all nested `/str/cnv/` dialogue tables). Only `en-us` assets are installed, so the `strings` table is **~973k rows**, all `en-us` (≈558k pre-#281 + ~415k conversation lines). The `locale` is read from the path, so dropping the `fr-fr`/`de-de` `.tor` archives in and re-extracting would populate them with no code change.

### 6a. The strings table is where quest/mission CONTENT lives (verified 2026-06-01)

Strings are keyed `(id1 = field slot, id2 = object's string_id)`; the FQN is `str.<domain>.<id1>.<id2>`. This — **not** the GOM payload — is the source of all quest text. **`str.qst` slot map:**

| `id1` slot | Holds | Count |
|---|---|---:|
| `88` | **Mission name** (the canonical named-mission universe) | 6,761 |
| `89–199` | **Objective** lines ("Speak to X", "Slay the Beast") | ~14,280 |
| `200–699` | **Journal / step description** narrative | ~23,600 |
| `>699` | reference text | ~650 |

The link to a quest object is `strings.id2 = objects.string_id` (same join `quest_descriptions` uses). Downstream tables built from this: `quest_name_tags`, `quest_text`, `mission_catalog`.

**Named-mission universe ≫ extracted objects:** there are **6,761** `str.qst.88` mission names vs the **1,514** `qst` objects kessel extracts (§2a). The ~5,000 extras (heroics, flashpoints, weeklies, dailies) exist only as named strings — now first-class in `mission_catalog` (keyed by `string_id`), but they carry no FQN/planet/class (those need the object). Coverage vs the Exarch oracle: 82% exact name match + superset size, no clear gaps.

### 6b. str namespaces (top)

`str.cnv` **414,893** (conversation dialogue lines — see §6d) · `str.itm` 241,606 · `str.abl` 182,842 · `str.qst` 45,379 · `str.npc` 40,328 (NPC names/titles, not dialogue) · `str.ach` 37,819 · `str.cdx` 7,671 · `str.tal` 2,341 · `str.gui` 210.

### 6d. Conversation dialogue lives in `/str/cnv/` (extracted as of #281)

The spoken/subtitle text for every conversation is in **per-conversation STBs under `/str/cnv/`** — 16,551 tables in the archive (5,768 en-us), the largest `/str/` category. The path maps onto the `cnv.*` object FQN (`extract_fqn_from_path`), so a line is `str.cnv.<conv path>.<id1>.<id2>`.

**Earlier (wrong) note corrected:** a prior revision of this atlas said "no `str.cnv` namespace; dialogue is not in `/str/`." That was an extraction artifact, not a fact — `should_extract_stb` only accepted root-level tables and rejected every nested one, so `/str/cnv/` was silently dropped. It is now extracted (the `cnv` module enables it). Real numbers: **414,893 dialogue lines across 5,671 conversations**, surfaced by the `conversation_lines` view (strips `str.` + trailing `.id1.id2` to recover `cnv_fqn`), joinable to conversations → quests via `conversation_quest_refs`.

The cnv **NODE** still has no dialogue — that's cinematics only (§3). Dialogue is in the `/str/cnv/` STBs, not the NODE.

### 6c. Quest classification: in the NAME, not the GOM enum

`qstActivityType` / `qstDifficulty` / `qstRewardsVisibility` / `qstEpisodeSeason` are class/prototype defaults and are **NOT serialized per quest** (0 CF40 occurrences of their hashes across all 1,514 quest payloads). The real signal is the **mission-name bracket tag** (`[HEROIC 2+]`, `[VETERAN]`, `[MASTER]`, `[FLASHPOINT]`, `[UPRISING]`, `[WEEKLY]`, `[DAILY]`) → parsed into `quest_name_tags`. Runtime-residual / not-in-archive (the ceiling): objective `count` ("0/10"), conversation flow, cross-quest prerequisites, planet-progression order (class story is all `qst.location.open_world.*`; arc edges are same-planet).

## 7. Where I am uncertain / earlier reflections may be wrong

These items need fresh verification rather than reliance on prior reflection snapshots:

- **PINF format**: prior reflection said `<u64 id><u8 flag><u8 unknown>` with flag=1 routing to cnv prototypes; live data shows flag bytes uniformly distributed across 0x00–0xFF. Routing interpretation likely wrong.
- **`.node` count present in archives**: the file-format catalog reflection says all 17,514 are real PROT files; an earlier audit said only ~10,735 resolve. A sample hash from the hash dictionary returned "not found in any .tor" via `dump_epp`. Needs a fresh walk: load the hash dict, attempt to resolve each of the 17,514 .node hashes, count present vs absent.
- **Bug-dropped prototype prefixes**: 10+ singleton prototypes are misnamed in the PBUK survey because the FQN extractor drops byte 0. Names like `urb`, `tion`, `eration`, `ashpoint` represent real prototypes (`Suburb`, something-`tion`, `Federation`/something, `Flashpoint`). The catalog count of 370 is approximate until the extractor is fixed.

## 8. Discovery tools used to produce this atlas

All under `kessel-discovery/src/bin/`:

| Binary | What it does |
|---|---|
| `survey_prefixes` | Counts FQN prefixes across all PBUK buckets. |
| `sample_per_prefix` | Pulls one sample object per prefix with payload-size + ASCII hint. |
| `catalog_singletons` | Inventories every zero-dot prototype (the 370 master tables). |
| `dump_epp` | Reads raw bytes of any file in any .tor archive by full resource path. |
| `extract_info_files` | Pulls `prototypes.info`, `buckets.info`, `scriptdef.list` to `/tmp/*.bin`. |
| `pinf_flag_histogram` | Builds a flag-byte histogram from `prototypes.info`. |

## 9. What this atlas is NOT

- Not a decoder. Each entry locates and labels; record-level field decoding is separate work.
- Not a roadmap. Picking which categories to extract next is a different decision than mapping where they are.
- Not authoritative beyond the snapshot date — counts shift with each SWTOR patch.
