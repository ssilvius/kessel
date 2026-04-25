# Changelog

All notable changes to kessel are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Cargo semver](https://doc.rust-lang.org/cargo/reference/semver.html).

## [Unreleased]

### Added

- quest_chain table populated from GUID refs in quest payloads, linking chain members by resolved u64 identifiers
- template_guid column on objects table, decoded from GOM header bytes 16-23 (kind-level template constant)
- quest_npcs table populated by resolving a:enc.* references in quest payloads to npc.* FQNs through encounter object payloads (closes #14)

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
