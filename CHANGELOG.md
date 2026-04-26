# Changelog

All notable changes to kessel are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Cargo semver](https://doc.rust-lang.org/cargo/reference/semver.html).

## [Unreleased]

### Added

- quest_chain table populated from `0xCF` big-endian GUID refs in quest payloads; fixes the zero-row bug from PR #11 where bytes were read as little-endian (closes #7)
- template_guid column on objects table, decoded from GOM header bytes 16-23 (kind-level template constant)
- quest_npcs table populated by resolving a:enc.* references in quest payloads to npc.* FQNs through encounter object payloads (closes #14)
- quest_rewards table populated by extracting `quest_reward_*` variable names from quest payloads (closes #24)
- quest_descriptions view exposing each quest's first journal entry (STB id1 200-600 range) -- mirrors the CSV "Mission Description" column
- bonus_missions view flattening `mpn.*.bonus.*` mission-phase objects with their parent quest FQN guess -- helps close the kessel/CSV row-count gap (#25)
- spawn_runtime_ids table populated from SPN-triple numerics in quest payloads -- bridges combat-log entity events to kessel's content GUIDs once the log format is verified. Closes #31.
- gui/planetaryconquest.stb and gui/galacticcommand.stb extracted into the strings table. Conquest theme names ("Total Galactic War", "The Trade Emporium", etc.) and "Invasion Bonus" category mappings now queryable. Closes #39.
- missions table unifying qst.* objects with mpn-prefix groupings. SWTOR's mission identity is encoded as either a qst.* object OR a unique path-prefix of mpn.* phases (alliance alerts, many class-story missions, etc. live only as the latter). Closes #34. Goes from 1,315 quest identities to ~3,950 mission identities.
- conquest_objectives table: structured view of `ach.conquests.*` (713 rows) with category, subcategory, and cadence parsed from FQN segments. Categories: chapter / class / crafting / event / flashpoint / galactic_seasons / location / operation / spvp / uprisings / quest / weekly. Cadence: weekly / daily / null. Closes #36.
- conquest_invasion_bonuses view exposing each "Invasion Bonus - <categories>" string from planetaryconquest as (id1, categories) rows. The theme-to-bonus rotation is server-side (per Sean: published as iCal feed); kessel publishes the static catalog of bonus category sets.
- conquest_theme_strings view: filtered planetaryconquest strings in the theme id1 range (300-360), excluding UI chrome. Themes have inconsistent name/description ordering in the source so the view leaves pairing to consumers.
- mission_npcs and mission_rewards tables aggregating NPC and reward extractions across each mission's phase tree. For qst-source missions, this is the quest's own payload. For mpn-prefix missions, this walks every `mpn.<prefix>.*` child phase. mpn-only missions (alliance alerts, class-story side missions like Mannett Point) now get their NPCs and rewards extracted -- they were silently zero before because populate_quest_npcs only iterated `kind='Quest'`.

### Fixed

- enc / spn / plc FQN prefixes added to extraction allowlist. quest_npcs was empty after every extraction because encounter objects were filtered out before populate_quest_npcs could resolve them.
- Extended quest_npcs resolution to three hops (quest -> enc -> spn -> npc). Encounter payloads contain spawn references, not NPC references directly; without the spawn-to-NPC step, encounter resolution found zero rows.
- String scanner recognises a third encoding pattern: `0xD2 0x01 <index> <len> <ASCII>` for array-element strings in encounter payloads. The previous heuristic produced truncated strings (e.g. `Gspn.location...ban` instead of the full FQN).
- mpn.* objects now classify as `kind='Phase'` (was `kind='Quest'`). Mission phases were inflating the Quest count 8x and polluting `quest_details` with phase-shaped rows that were never real missions. New `phases` view exposes them. Closes #23.
- Spawn-prefix fallback in populate_quest_npcs: encounters that reference a base spawn name (e.g. `spn.X.multi.isen`) which the engine resolves at runtime to variants (`isen_no_weapon`, `isen_captured`) now resolve to the underlying NPC via prefix-match. Closes the_devoted_ones case from #27.
- string_id type-marker decoder tries 3-byte big-endian before 4-byte little-endian. The previous LE32-first order was poisoning achievement IDs by absorbing a trailing 0x00 separator byte. Achievement string_id coverage 42% -> 99.9%; conquest objectives specifically went 0% -> 100% (687/687). No regression on quests/items/talents. Closes #37.

### Removed

- populate_quest_chain (PR #11's 0xCF GUID-ref hypothesis). Brute-force search confirmed quest content GUIDs are not cross-referenced statically -- the function produced zero rows on every real extraction. Closes #19. The `quest_chain` table is retained for future link mechanisms (e.g. mpn-derived edges).

## [0.0.5] - 2026-04-02

First tagged release. Extracts structured SWTOR game data from .tor archives to SQLite.

### Added

- MYP archive reader with zstd/zlib decompression
- PBUK/DBLB container parser for GOM objects
- Hash dictionary auto-download from Jedipedia
- STB string table extraction with locale support
- Grammar rules engine for cleaning SWTOR template syntax
- DDS to WebP icon conversion with deduplication
- Quest classification module: mission type, faction, planet, class code, companion class
- Quest extraction tables: quest_details, quest_npcs, quest_phases, quest_prerequisites, quest_chain
- Gift item classification from FQN patterns
- FQN-based string_id extraction (99.8% coverage, up from 42%)
- Batch SQLite inserts with transaction wrappers
- Content filtering with --unfiltered bypass
- CI: check, test, clippy, fmt
- Release builds: Linux x86_64, Windows x86_64, macOS Apple Silicon
