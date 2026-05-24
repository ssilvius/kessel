# Probe: dropped PBUK prefixes

Sample-byte inspection of every major PBUK FQN prefix dropped by `kessel/src/main.rs:740`'s 19-prefix whitelist. **Locates and labels what each prefix contains**; does not decode record fields.

Method: pull one canonical object per prefix via `dump_npp -p <prefix>`, scan the payload for plaintext strings + class-template hi32 markers, infer the semantic role from FQN, embedded strings, and cross-references.

Snapshot: 2026-05-24 against `apesVersion=2961.1.1`. 608,131 PBUK objects total across 451 prefixes; whitelist accepts 19 (~316K extracted), drops 432 (~292K). The 11 largest dropped prefixes account for ~150K of the missing objects.

## Top dropped prefixes (bulk categories)

### `hyd.*` — 34,264 records — gameplay event handlers

Sample: `hyd.location.nar_shaddaa.mob.hub2.green.rep1.room02.poi00_red_light_entrance.pth_arrival_techsmith_01`

Embedded plaintext: `On Arrive`, `npc.location.nar_shaddaa.mob.humanoid.neutral.red_light_techsmith`, `senc.location.nar_shaddaa.mob.hub2.green.rep1.room02.poi00_red_light_entrance.poi00_red_light_entrance_staged01_sf01`

**Role**: scene-event hook records. Each binds:
- a TRIGGER EVENT (`On Arrive`, `On Click`, `On Death`, etc — name appears as plaintext, sometimes twice indicating "event + condition")
- a TARGET REFERENCE (NPC FQN, placeable FQN, encounter FQN)
- a STAGING SPAWN (the `senc.*` reference indicates which encounter scaffolding fires)

34,264 of these = per-zone NPC arrival behaviors, click-to-interact logic, death triggers, encounter spawns. Currently absent from spice; kessel has no event-handler concept. Quest/conversation/encounter linkage would be significantly enriched by extracting these.

### `cnd.*` — 20,065 records — named boolean conditions

Sample: `cnd.itm.has_item.lots.armor.bh_tro_dps.conquest.ilvl_0162.premium.armor_wrists`

Embedded plaintext: `str.cnd` (string-table family for condition labels)

**Role**: reusable boolean predicates. The FQN itself encodes the predicate type (`cnd.itm.has_item.*`, `cnd.qst.*`, etc — first segment after `cnd.` is the predicate family). Each is parameterized — `has_item.lots.armor.bh_tro_dps.conquest.ilvl_0162.premium.armor_wrists` is "has the BH Trooper DPS Conquest ilvl-162 premium wrist piece."

20,065 named conditions reused across:
- Quest objectives ("this step requires X")
- Ability conditions ("this ability needs X")
- Spawn triggers
- Dialog node prerequisites (likely the source of parsely's `fullCondition` expressions)

Kessel's current `populate_quest_prerequisites_graph` derives flag-graph state by inference; cnd.* records would be the authoritative predicate definitions.

### `npp.*` — 20,049 records — NPC build/spec packages

Sample: `npp.location.makeb.mercenaries.infantry_bms_elite`

Embedded plaintext: `bms` (codename, possibly "battlefield mercenary spec"), `human_republic_medium_male_08` (appearance archetype)

**Role**: NPC LOADOUT TEMPLATES. Distinguishes from:
- `npc.*` — NPC instance definitions (name, FQN, role) — extracted
- `spn.*` — spawn placement records (where to put the NPC) — extracted
- `npp.*` — the BUILD package (which stats, which gear, which appearance archetype, which abilities) — **dropped**

20,049 build templates. Without these, per-NPC stat/gear/loadout information isn't queryable. Boss tuning, mob difficulty scaling, ambient NPC outfitting all live here.

### `dyn.*` — 13,453 records — dynamic boss-fight placeables

Sample: `dyn.operation.iokath.boss.scyva.combat.railgun_cove_blue`

Embedded plaintext: art ref `/art/static/area/all_all/item/tech/all_item_tech_library_panel_02_works_white.gr2`, state names `State_0`, `State_1`, `State_2`, prop name `cove_wall_light_state_0_1`

**Role**: combat-time STATE-MACHINE OBJECTS. Each represents a multi-state interactable in a boss fight or set piece (railgun covers, panel lights, conduit interactions). Drives boss mechanics — "click this panel to enable the railgun cove," "destroy this conduit to disable the boss's shield."

13,453 of these = the mechanical objects boss-encounter guides reference by name. Currently dropped; would surface boss-mechanic objects for ops-guide content.

### `stg.*` — 7,796 records — cutscene/mission staging

Sample: `stg.location.hoth.class.spy.supply_cache_a`

Embedded plaintext: `UNINITIALIZED`, `CUSTOM`, self-FQN reference

**Role**: stage definitions for class-story cutscenes, mission key moments, conversation-driven scene transitions. Lighter than `enc.*` (encounters) — these are NARRATIVE staging, not combat encounters. Each binds the scene's spawn set, camera/animation triggers, and player-spawn position.

7,796 of these = the cinematic moment graph. Filed under prior issue #134 as the "stages table" foundation (PR d804a30 on the abandoned branch); has schema-only foundation, no populator.

### `apn.*` — 4,723 records — animation packages

Sample: `apn.npc.qtr.1x1.raid.karaggas_palace.enemy.trash.factory.difficulty_1.assassin_droid`

Embedded plaintext: dense byte arrays that look like compressed sequence data (`!!""##$$%%&&...`)

**Role**: per-NPC ANIMATION GRAPHS — the sequence of animation triggers a specific NPC archetype uses (idle, combat states, special-attack cues). The compressed-looking byte runs are likely interpolation tables or animation event lists.

Linked to `apc.*` (appearance) and `npp.*` (build) — together they describe an NPC's full visual + behavior package.

## Small dropped prefixes (worth labeling)

### `cos.*` — 496 — cosmetic NPC archetype tags

Samples: `cos.location.taris_imperial.mob.hub1.poi06_bomber_command_post.ambient01.strong_r_01` ("Humanoid - Ambient"), `cos.location.tatooine.mob.hub1.green.rep.poi04_outpost_dreviad.refurbished_militia_droid` ("Droid - Battledroid")

**Role**: ambient-NPC visual archetype tags. A short label that groups visually-similar NPCs (humanoid ambient, battledroid, etc.) for art/cosmetic purposes.

### `pcs.*` — 211 — player character species presets

Samples: `pcs.jedi_knight.female.rattataki_legacy`, `pcs.imperial_agent.male.cathar`, `pcs.bounty_hunter.male.cathar`

**Role**: PLAYER CHARACTER PRESETS keyed by (class × gender × species [× variant]). Each preset likely holds a default appearance, starting gear set, opening cinematic anchor. 211 = covers every supported (class, gender, species) combination plus legacy / cartel-purchased species variants.

Useful for character-creation context and class-story start-state documentation.

### `nco.*` — 184 — NPC companions (era-tagged)

Samples: `nco.companions_original.warrior.broonmark`, `nco.companions_kotet.shae_vizla`

Embedded plaintext: `str.nco` (string-table family)

**Role**: companion-character master records, tagged by expansion era (`companions_original` = launch era, `companions_kotet` = Knights of the Eternal Throne era, likely `companions_kotfe`, `companions_macrobinoculars`, etc.). Each holds the companion's class assignment, base personality, gift preferences.

Different from `chrCompanionTable_Prototype` (the singleton master list) — these are per-companion records. Comparable to NPC instances but specialized for companions.

### `mrp.*` — 163 — mount/reward packages

Samples: `mrp.galactic_seasons.season_5.mouse_droid`, `mrp.daily_area.iokath.empire.mounted_turret`, `mrp.dynamic_events.dantooine.caves_biome.level_3.under_pressure.mouse_morph`

**Role**: MOUNTED-VEHICLE/REWARD packages for time-limited events. Includes event mounts (mouse droid), daily-area special mounts (Iokath turret), expansion event vehicles, MTX rewards. The FQN encodes the event source (`galactic_seasons.season_5`, `daily_area.iokath`, etc.).

Currently kessel has no `mounts` table — `mntMountInfoPrototype` (singleton) + these per-mount records would provide a full mount catalog.

### `ipp.*` — 116 — item paint/pattern prototypes

Samples: `ipp.custom.sow.progresson.ge_a13_purple_holo.legs`, `ipp.pvp_seasons.season9.armor_prestige_variant.chest`

Embedded plaintext: `mtx/gear_flourish/pvp_gearfx_spacepirate_legs_purple_bfs` (gear FX reference)

**Role**: gear COSMETIC FLOURISH presets (PvP season armor variants, custom dye reskins, holographic accents). Each binds a gear slot + cosmetic FX overlay (purple holo, gear_flourish FX). Used by season rewards and cartel market customizations.

## Bug-pattern garbage prefixes (10 of them)

These appear in the survey as if they were real prefixes, but they're **artifacts of the PBUK FQN extractor dropping byte 0** on roughly 10 singleton prototype names:

| Garbage prefix | Likely real prefix | Sample |
|---|---|---|
| `cation` (41) | `Location` | `cation.open_worlds.class.sith_warrior.chapter_3.reallocation.event_draagh` |
| `ami` (19) | likely `Family` | `ami.leg` |
| `urb` (2) | `Suburb` | `urb.test.aldaza.farfaraway` |
| `eration` (9) | `Federation` | `eration.oricon.palace.boss.tyrans.tile_logic.neutralize_fall_debuff` |
| `liance` (6) | `Alliance` | `liance.desperate_defiance.shared.boss.rikan_kateen.rikan_p2_escalation_2` |
| `mpanion` (6) | `Companion` | `mpanion.ventures.basilisk_prototype.tank.unique_aoe_attack.maelstrom_impact` |
| `aceables` (2) | `Placeables` | `aceables.pvp.voidstar.plant_bomb/10/0` |
| `dex` (2) | `Codex` | `dex.persons.ilum.supreme_commander_rans` |
| `neric` (2) | `Generic` | `neric.destroyable.republic.tech.rep_turret_usable` |
| `ashpoint` (5) | `Flashpoint` | `ashpoint.secrets_of_the_enclave.bosses.boss3.golah.flurry` |
| `th_warrior` (2), `unty_hunter` (2) | `sith_warrior`, `bounty_hunter` | per-class shared FQNs corrupted |

These are not separate categories — they're real prototypes whose first byte got chopped during PBUK parsing. Fix queued as "tighten the FQN extractor" side-quest. Until fixed, the singleton catalog count (370) is approximate.

## Discovery binary

```bash
cargo build --release -p kessel-discovery --bin dump_npp
./target/release/dump_npp -i ~/swtor/Assets -H /tmp/hashes_filename.txt -p <prefix>
```

`dump_npp` walks PBUK buckets and dumps the first object matching the prefix as hex + extracted strings. Useful for cheap "what is this kind?" probes without committing to a full decoder.

## What this changes for the atlas

The atlas (`docs/CORPUS_ATLAS.md`) labels each prefix at the count level. This probe doc anchors what each one *actually contains* at the structural level — sufficient detail to scope an extractor for any one of them without further investigation.

Combined with the dis-payload probe (`docs/probes/dis-payload-format.md`), the kessel scope problem is now:

- **PBUK prefix categories**: mapped (this doc + atlas)
- **Singleton master tables**: catalogued (atlas section + `catalog_singletons` binary)
- **Specific high-value formats**: probed (dis.* in detail; hyd/cnd/npp/dyn/stg at category level)
- **File-extension categories**: identified by magic bytes (atlas section §5)

Next decision point is *which* of these to convert into shipping extractors — that's a separate question from mapping where they live.
