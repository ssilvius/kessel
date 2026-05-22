# Singleton Prototype Catalog

PBUK bucket objects come in two shapes:

1. **Per-instance GameObjects** (kessel today) -- namespaced dotted FQNs like `abl.sith_warrior.skill.rage.ravage`, `itm.tactical.sow.juggernaut.grit_teeth`. Many of each kind. Indexed by GUID + FQN. 19-prefix whitelist in `should_extract_object`.
2. **Singleton prototype tables** (kessel does not read) -- zero-dot FQNs like `utlShipInfoPrototype`, `mntMountInfoPrototype`, `itmRatingTablePrototype`. Exactly one of each. Each payload is a TABLE -- an array of records describing ships / mounts / companions / conquest events / etc.

This catalog inventories the **370 singletons** in 7.8.1.c. Companion of #54 (`Survey: catalog all GOM object type prefixes`).

Source: `target/release/catalog_singletons -i ~/swtor/Assets -H ~/swtor/data/hashes_filename.txt > docs/prototypes-catalog.tsv`. Re-run after each archive version.

Reflections: `019df4f1` (GSF Phase 0), `019df4f6` (GSF prototype contents), `019df4fd` (architectural framing), `019df4f2` (research pattern).

---

## Size distribution

| Bucket | Count |
|---|---|
| >= 1 MB | 8 |
| 100 KB - 1 MB | 22 |
| 10 KB - 100 KB | 92 |
| 1 KB - 10 KB | 119 |
| < 1 KB | 128 |
| FQN parse oddities (truncated / garbage) | ~20 |

## High-value prototypes by domain

Decode-feasibility:
- **High**: payload has many readable ASCII strings (names, FQN refs, paths) -- per-record schema visible.
- **Medium**: structured CF E0 / CF 40 markers but mostly numeric -- decodable with a per-byte schema map.
- **Low**: no markers, opaque numeric blob -- needs reverse engineering.

### Items / gear

| FQN | Size | E0 refs | Strings | Decode | Likely contents |
|---|---:|---:|---:|---|---|
| `itmBudgetedAttributesPrototype` | 3.7 MB | 0 | 315 | medium | stat-budget ladder per item rating (canonical stat-curve table) |
| `itmSetTablePrototype` | 647 KB | 19499 | 1060 | medium | item-set master table (canonical join for sets) |
| `itmModifierSetTablePrototype` | 192 KB | 12 | 431 | medium | item modifier sets / affixes |
| `itmAppearanceDatatable` | 394 KB | 0 | 12862 | high | appearance data, art paths |
| `itmAugmentCostsPrototype` | 5 KB | 0 | 0 | medium | augment vendor pricing |
| `itmEnhancementInfoPrototype` | 4 KB | ? | ? | medium | enhancement metadata |
| `itmSetBonusesPrototype` | small | ? | ? | medium | canonical set-bonus master list (#110 derived a partial view from FQNs; this is the source of truth) |
| `itmRatingTablePrototype` | small | ? | ? | medium | item rating curve |
| `itmSearchTagsPrototype` | small | ? | ? | medium | tag-based search categories |
| `ahItemSlotCategoriesPrototype` | small | ? | ? | medium | **item slot enum** (resolves #59 slot field) |
| `ahItemCategoriesPrototype` | small | ? | ? | medium | top-level item taxonomy |
| `ahItemSubCategoriesPrototype` | small | ? | ? | medium | subcategory taxonomy |
| `ahItemStatsPrototype` | 127 B | ? | ? | medium | stat enum |
| `chrGearScorePrototype` | small | ? | ? | medium | item rating curve / GearScore |
| `chrOutfitCostsPrototype` | small | ? | ? | medium | outfit designer costs |

### Quests / story

| FQN | Size | E0 refs | Strings | Decode | Likely contents |
|---|---:|---:|---:|---|---|
| `qstRewardsInfoPrototype` | 2.8 MB | 121168 | 5093 | high | quest reward master table (relates to #15 quest_objectives, #67 flag-graph) |
| `haredInformationPrototype` (truncated `SharedInformation...`) | 941 KB | 19333 | 10664 | high | quest companion alert refs (`qst.alliance.alerts.companions.adon.adon_recruitment` etc.) |
| `lgcAchievementEventsPrototype` | 1.0 MB | 92395 | 3906 | medium | achievement event registry |
| `achCategoriesTable_Prototype` | 308 KB | 11076 | 2401 | high | achievement categorization (cdx.planets.tatooine, story, worldarc) |
| `achRewardsTable_Prototype` | small | ? | ? | medium | achievement reward bindings |
| `cdxCategoryTotalsPrototype` | small | ? | ? | medium | codex completion thresholds |
| `cdxCompletionBonusPrototype` | 83 B | 0 | ? | low | completion bonus table |
| `cdxBitToFQNPrototype` | 47 KB | 3946 | 153 | medium | codex bit -> FQN reverse index |

### GSF (Galactic Starfighter)

Verified Phase 0 -- see `019df4f1`, `019df4f6`.

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `utlShipInfoPrototype` | 344 B | 8 | 0 | medium | 8 ship classes (Strike/Scout/Gunship/Bomber x 2 factions). Variant lookup needs follow-up decode. |
| `scFFComponentUpgradesCostPrototype` | 62 KB | many | 0 | high | 5-tier requisition ladder per component (verified: 500, 1250, 2500, 5000, 7500) |
| `scFFComponentsCostPrototype` | 18 KB | many | 0 | medium | initial component purchase cost |
| `scffCrewPrototype` | 17 KB | 276 | 186 | high | full GSF crew table; names visible (`spvp_Crew_icon_<name>`) |
| `scffCrewPackagesPrototype` | 835 B | ? | ? | medium | crew packaging |
| `scFFColorSwatchesPrototype` | 37 KB | ? | ? | medium | paint swatches |
| `scFFColorOptionsCostPrototype` | 1.4 KB | ? | ? | medium | paint costs |
| `scFFPatternsDefinitionProtoype` | 11 KB | ? | ? | medium | pattern definitions (note: typo `Protoype` is canonical in archive) |
| `scFFPatternsCostPrototype` | 2.6 KB | ? | ? | medium | pattern costs |
| `scFFPatternsTextureDataProtoype` | small | ? | ? | low | pattern texture refs |

### Cartel market / cosmetics / collections

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `colCollectionItemsPrototype` | 2.3 MB | 19443 | 12333 | high | full collections catalog (Mtx.Season3.Bikini_V02, mtx_platter_galactic_seasons, ...) |
| `colCollectionCategoriesPrototype` | 225 KB | 6 | 1857 | high | collection categories (mtx_platter_armor, mtx_explorer_core_worlds) |
| `colCollectionSourcesPrototype` | 40 KB | 15 | 325 | medium | collection sources (where items come from) |
| `mtxStorefrontInfoPrototype` | 1.8 MB | 32216 | 10708 | medium | cartel market storefront |
| `mtxStashItemPlateDataPrototype` | 103 KB | 2638 | 2730 | high | stash plates (mtx.season2.ald_npc.house_organa_v02) |
| `mtxUnlockMappingPrototype` | 74 KB | 16 | 1936 | medium | unlock mappings |
| `chrCharacterToysPrototype` | small | ? | ? | medium | toy items / fluff |

### Mounts / pets / vanity

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `mntMountInfoPrototype` | 888 KB | 3348 | 16057 | high | **full mount catalog** (kessel has zero mounts today) |
| `ablVanityPetsPrototype` | 24 KB | 2450 | 110 | medium | vanity pet registry |
| `chrCharacterToysPrototype` | small | ? | ? | medium | character toys |

### Conquest / seasons / weekly content

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `cnqConquestInfoPrototype` | 141 KB | 13209 | 613 | high | conquest events with names (RakghoulCorellia, cdx.strongholds.conquest.rakghoul_cor) |
| `cnqPlanetInfoPrototype` | small | ? | ? | medium | planet weekly conquest schedule |
| `cnqAchGroupPrototype` | small | ? | ? | medium | achievement groups per conquest |
| `lgcGalacticSeasonsPrototype` | 92 KB | 3588 | 323 | medium | galactic season rewards / objectives |
| `lgcDailyLoginCalendarPrototype` | small | ? | ? | medium | daily login rewards calendar |
| `lgcDailyLoginSubscriberRewardsPrototype` | small | ? | ? | medium | sub login rewards |
| `lgcPvPSeasonsPrototype` | small | ? | ? | medium | PvP season config |

### Character / class / companion

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `chrCompanionInfo_Prototype` | 87 KB | 3358 | 464 | medium | companion master records |
| `chrCompanionTable_Prototype` | small | ? | ? | medium | companion table (relates to existing companion handling) |
| `chrCompanionSpecMap_Prototype` | small | ? | ? | medium | companion spec (tank/heal/DPS) mapping |
| `chrAdvancedClassDataPrototype` | small | ? | ? | medium | discipline / advanced class data (#80 GSF talent residual may be here) |
| `chrClassListingPrototype` | 38 KB | 4244 | 179 | medium | class listing |
| `chrBackgroundTablePrototype` | 25 KB | 632 | 405 | medium | character creation backgrounds |
| `chrCharacterStoryClassPrototype` | small | ? | ? | medium | story class mapping |
| `chrSpeciesScalePrototype` | small | ? | ? | medium | species body scaling |
| `chrPaidPermissionDefsTablePrototype` | 405 KB | 17 | 3588 | high | paid permissions (paid_permission.abl.early_access.mount.rank1) |
| `chrCurrencyTablePrototype` | small | ? | ? | medium | currency definitions |

### Combat / stats

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `cbtArmorPerLevel` | 279 KB | 0 | 0 | low | armor scaling per level (numeric) |
| `cbtShieldPerLevel` | 241 KB | 0 | 337 | low | shield scaling per level |
| `cbtStandardDamageInfo` | small | ? | ? | medium | damage standard tuning |
| `cbtStandardHealingInfo` | small | ? | ? | medium | healing standard tuning |
| `cbtStandardRatingInfo` | small | ? | ? | medium | rating-to-stat conversion |
| `cbtArmorTablePrototype` | small | ? | ? | medium | armor table |
| `mplifierInstancesPrototype` (truncated `Amplifier...`) | 111 KB | 0 | 2202 | high | amplifier instances (amp.test.jr.armor_penetration_25) |
| `statAmplifiersPrototype` | 12 KB | 13 | 197 | medium | amplifier base data |
| `statAmplifierPackagesPrototype_Client` | small | ? | ? | medium | amplifier packages |
| `tmTierInfoPrototype` (truncated `itmTier...`?) | small | ? | ? | medium | item tier info |

### Guild / flagship

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `gldFlagshipPrototype` | 21 KB | ? | ? | medium | guild flagship config |
| `gldLevelInfoPrototype` | small | ? | ? | medium | guild level rewards |
| `gldPerkInfoPrototype` | 59 KB | 1497 | 427 | medium | guild perks |
| `gldHeraldryInfoPrototype` | small | ? | ? | medium | guild heraldry |
| `gldTagsInfoPrototype` | small | ? | ? | medium | guild tags |

### Other

| FQN | Size | E0 refs | Strings | Decode | Contents |
|---|---:|---:|---:|---|---|
| `tagTablePrototype` | 450 KB | 28 | 7083 | high | tag system (tag.abl.qtr.flashpoint.rishi.flashpoint_2.mob.boss.boss_1.spy.in_stealth) |
| `decorationsPrototype` | 517 KB | 50865 | 2224 | medium | stronghold decorations |
| `prfDebugSchematicMapPrototype` | 380 KB | 40192 | 1603 | low | crafting schematic debug map |
| `prfSchematicVariationsPrototype` | 115 KB | 8441 | 543 | medium | schematic variations |
| `pkgTrainingCostTablePrototype` | small | ? | ? | medium | training cost (legacy / class trainer) |
| `mmsChallengesPrototype` | small | ? | ? | medium | matchmaker challenges |
| `mmsMatchmakersPrototype` | small | ? | ? | medium | matchmaker config |

---

## FQN parse oddities

The PBUK FQN extractor truncates the leading character on some objects. Examples:

| In catalog | Likely real name |
|---|---|
| `hipCostPrototype` | `shipCostPrototype` |
| `haredInformationPrototype` | `SharedInformationPrototype` |
| `mplifierInstancesPrototype` | `AmplifierInstancesPrototype` |
| `oiceEffectsTablePrototype` | `VoiceEffectsTablePrototype` |
| `chedulePrototype` | `SchedulePrototype` |
| `edCenterPrototype` | `MedCenterPrototype` |
| `erkPrototypeMap` | `PerkPrototypeMap` |
| `nlockPrototypeMap` | `UnlockPrototypeMap` |
| `indpointsPrototype` | `WindpointsPrototype` ? |
| `ayer`, `ooper`, `oid`, `n`, `p`, `r`, `c`, `m`, `d`, `x` | leading-byte garbage from non-prototype objects -- noise, ignore |

When wiring decoders, normalize against the actual on-archive FQN bytes rather than these displayed names. Worth a follow-up to tighten the FQN extractor (filed as a comment on #54 if not already).

## Decode priority recommendations

Highest-leverage prototypes to decode first, ranked by **(consumer need) x (decode confidence) x (size of unlocked dataset)**:

1. **`scFFComponentUpgradesCostPrototype`** -- huttspawn placeholder consumers exist, structure verified, 5-tier req ladder per component. Proof-of-pipeline for the prototype-table extractor.
2. **`mntMountInfoPrototype`** -- 16K strings, 3.3K records, kessel has zero mounts today. New `mounts` table.
3. **`itmBudgetedAttributesPrototype`** -- the canonical stat-budget curve. Underpins all gear stat math. Likely fixes a class of huttspawn item-stat questions in one shot.
4. **`itmSetTablePrototype` + `itmSetBonusesPrototype`** -- canonical set-bonus join. #110 derived a partial view from FQNs; these prototypes are the source of truth.
5. **`ahItemSlotCategoriesPrototype` + `ahItemCategoriesPrototype`** -- canonical slot enum and item taxonomy. Closes the slot-field branch of #59 without per-payload archaeology.
6. **`scffCrewPrototype`** -- GSF crew table, names visible in payload, decoder is straightforward.
7. **`colCollectionItemsPrototype`** -- 12K strings, full collection catalog. New `collections` table.
8. **`cnqConquestInfoPrototype` + `cnqPlanetInfoPrototype`** -- conquest event registry. New `conquest_events` table.
9. **`chrCompanionTable_Prototype`** -- companion master table. May supplement existing companion handling without replacing it.
10. **`utlShipInfoPrototype`** -- 8 ship classes, but ship VARIANTS need a follow-up decode pass; do this after the proof-of-pipeline lands.

Schema-locked stance for `objects` / `strings` / existing side tables holds throughout. Each new prototype adds a dedicated table; nothing existing changes shape.

## Raw inventory

Full TSV at `docs/prototypes-catalog.tsv`. Columns: `FQN`, `PAYLOAD_SIZE`, `CF_E0_COUNT` (content-GUID refs), `CF_40_COUNT` (template refs), `STRING_COUNT`, `FIRST_32_BYTES`, `STRING_SAMPLES`.
