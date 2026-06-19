//! Schema DDL + the `init_schema` table-creation step.
//!
//! Holds only the CORE tables (objects, strings, singletons) + the schema
//! dispatcher. Every domain owns its own tables in its module's create_tables.

use anyhow::Result;
use rusqlite::Transaction;

/// Create the core tables (objects/strings/singletons). Domain tables are
/// created by each domain module's `create_tables`. Idempotent (IF NOT EXISTS).
pub(crate) fn create_core_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
            -- Raw game objects table.
            --
            -- Identity columns:
            --   game_id     PK   = sha256(fqn:guid)[0:16]. Unique per
            --                      object-instance per extraction. Every join
            --                      and FK in this DB targets this column.
            --                      Shifts on patch (because guid does); the
            --                      PK is the change-signal column by design.
            --   stable_id        = sha256(fqn)[0:16]. Stable across patches;
            --                      unique only post-`mark_canonical_by_fqn`.
            --                      Used for cross-version delta joins.
            --   payload_hash     = sha256(payload_bytes)[0:16]. Not an
            --                      identity. Detects "did this object's data
            --                      change" between extractions when joined
            --                      to stable_id.
            --   guid             = raw 16-char content GUID from GOM header.
            --                      Kept as a forensic / change-signal column.
            --   is_canonical     = 1 for the row chosen by
            --                      `mark_canonical_by_fqn`'s quality
            --                      heuristic, 0 for inferior variants. Lossless
            --                      replacement for the old DELETE-based dedup;
            --                      consumers filter `WHERE is_canonical = 1`.
            CREATE TABLE IF NOT EXISTS objects (
                game_id TEXT PRIMARY KEY,
                stable_id TEXT NOT NULL,
                payload_hash TEXT NOT NULL,
                guid TEXT NOT NULL,
                template_guid TEXT NOT NULL DEFAULT '',
                fqn TEXT NOT NULL,
                kind TEXT NOT NULL,
                icon_name TEXT,
                string_id INTEGER,
                for_export INTEGER NOT NULL DEFAULT 1,
                is_canonical INTEGER NOT NULL DEFAULT 1,
                version INTEGER NOT NULL DEFAULT 0,
                revision INTEGER NOT NULL DEFAULT 0,
                json TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_objects_fqn ON objects(fqn);
            CREATE INDEX IF NOT EXISTS idx_objects_stable_id ON objects(stable_id);
            CREATE INDEX IF NOT EXISTS idx_objects_payload_hash ON objects(payload_hash);
            CREATE INDEX IF NOT EXISTS idx_objects_guid ON objects(guid);
            CREATE INDEX IF NOT EXISTS idx_objects_kind ON objects(kind);
            CREATE INDEX IF NOT EXISTS idx_objects_for_export ON objects(for_export);
            CREATE INDEX IF NOT EXISTS idx_objects_is_canonical ON objects(is_canonical);
            CREATE INDEX IF NOT EXISTS idx_objects_string_id ON objects(string_id);
            CREATE INDEX IF NOT EXISTS idx_objects_icon_name ON objects(icon_name);
            CREATE INDEX IF NOT EXISTS idx_objects_template_guid ON objects(template_guid);

            -- Localized strings table (from STB files)
            CREATE TABLE IF NOT EXISTS strings (
                fqn TEXT PRIMARY KEY,          -- Full FQN: "str.abl.sith_inquisitor.skill.corruption.innervate"
                locale TEXT NOT NULL,          -- Locale: "en-us"
                id1 INTEGER NOT NULL,          -- STB ID1
                id2 INTEGER NOT NULL,          -- STB ID2 (links to objects.string_id)
                text TEXT NOT NULL,            -- Display text (grammar-cleaned: <<N>> templates stripped)
                text_raw TEXT,                 -- Raw STB text before grammar.clean(); NULL when identical to `text`. Preserves <<N>> positional templates for stat-value/label anchoring.
                version INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_strings_id2 ON strings(id2);

            -- guid -> string_id bridge for objects SEEN during extraction but
            -- DROPPED (unnamed / non-whitelisted FQN) and so absent from
            -- `objects`. Items reference such objects by guid (e.g. a relic's
            -- proc-buff ability via field 0x2d7b8786) and the objects' effect
            -- strings exist in `strings`; this table is the missing link from
            -- the referencing item to that string. #308.
            CREATE TABLE IF NOT EXISTS object_string_refs (
                guid      TEXT PRIMARY KEY,
                string_id INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_object_string_refs_sid ON object_string_refs(string_id);

            -- Typed views for convenience.
            -- Post-#23: kind='Quest' includes only qst.* objects.
            -- Mission phases (mpn.*) are kind='Phase' -- see `phases` view.
            -- Typed views filter to canonical rows. Inferior FQN variants
            -- live in `objects` with is_canonical = 0 for delta tooling and
            -- forensics; the views give consumers the deduped set.
            CREATE VIEW IF NOT EXISTS quests AS
                SELECT * FROM objects WHERE kind = 'Quest' AND is_canonical = 1;

            CREATE VIEW IF NOT EXISTS phases AS
                SELECT * FROM objects WHERE kind = 'Phase' AND is_canonical = 1;

            -- Conquest invasion-bonus mappings: each row is a string like
            -- "Invasion Bonus - Flashpoints, Warzones" describing the bonus
            -- category set highlighted by some conquest theme. The theme to
            -- bonus pairing is engine-driven (server-side rotation); the
            -- bonus catalog itself is static and lives here.
            CREATE VIEW IF NOT EXISTS conquest_invasion_bonuses AS
                SELECT id1, locale, substr(text, length('Invasion Bonus - ') + 1) AS categories
                FROM strings
                WHERE fqn LIKE 'str.gui.planetaryconquest%'
                  AND text LIKE 'Invasion Bonus - %';

            -- Conquest theme strings. Heuristic filter: planetaryconquest
            -- entries that aren't UI chrome. Theme-name vs theme-description
            -- pairing is left to consumers since the source pairing is
            -- inconsistent (sometimes name, sometimes description first).
            CREATE VIEW IF NOT EXISTS conquest_theme_strings AS
                SELECT id1, locale, text
                FROM strings
                WHERE fqn LIKE 'str.gui.planetaryconquest%'
                  AND id1 BETWEEN 300 AND 360
                  AND text NOT LIKE 'Invasion Bonus - %'
                  AND text NOT LIKE '%not authorized%'
                  AND text NOT LIKE '%Next Objective%'
                  AND text NOT LIKE '%Guild Rewards%'
                  AND text NOT LIKE '%Guild Flagship%'
                  AND text NOT LIKE '%not a member of a guild%'
                  AND text NOT LIKE '%currently in review%'
                  AND text NOT LIKE '%Guild Conquest point%'
                  AND text != '%';

            CREATE VIEW IF NOT EXISTS abilities AS
                SELECT * FROM objects
                WHERE (kind = 'Ability' OR fqn LIKE 'abl.%')
                  AND is_canonical = 1;

            CREATE VIEW IF NOT EXISTS items AS
                SELECT * FROM objects
                WHERE (kind = 'Item' OR fqn LIKE 'itm.%')
                  AND is_canonical = 1;

            CREATE VIEW IF NOT EXISTS npcs AS
                SELECT * FROM objects
                WHERE (kind = 'Npc' OR fqn LIKE 'npc.%')
                  AND is_canonical = 1;















































            -- Quest descriptions: first journal entry per quest, surfaced as
            -- a view over the strings table. Mirrors the CSV's "Mission
            -- Description" column. Per the design doc, journal text is at
            -- STB id1 200-600 range; the first entry is the description.
            CREATE VIEW IF NOT EXISTS quest_descriptions AS
                SELECT
                    o.fqn AS quest_fqn,
                    s.text AS description
                FROM objects o
                JOIN strings s ON s.id2 = o.string_id
                WHERE o.kind = 'Quest'
                  AND o.is_canonical = 1
                  AND s.id1 BETWEEN 200 AND 600
                  AND s.id1 = (
                      SELECT MIN(s2.id1) FROM strings s2
                      WHERE s2.id2 = o.string_id AND s2.id1 BETWEEN 200 AND 600
                  );

            -- Bonus missions flattened from mpn.*.bonus.* phases. The CSV
            -- treats these as separate mission rows; in GOM data they are
            -- mission phases of a parent quest. This view exposes them
            -- with parent FQN for editorial/CSV-style queries.
            CREATE VIEW IF NOT EXISTS bonus_missions AS
                SELECT
                    o.fqn AS bonus_fqn,
                    -- Parent quest FQN: drop the trailing `.bonus.<name>`
                    -- and any segments after `.bonus`. The mpn.* prefix
                    -- swaps to qst.* for the parent.
                    'qst.' || substr(
                        o.fqn,
                        5,
                        instr(o.fqn, '.bonus.') - 5
                    ) AS parent_quest_fqn_guess
                FROM objects o
                WHERE o.fqn LIKE 'mpn.%.bonus.%'
                  AND o.is_canonical = 1;

































            -- PBUK singleton prototypes (#171): one row per zero-dot PBUK
            -- object. These are master tables / config blobs the game references
            -- by FQN (tagTablePrototype, colCollectionItemsPrototype,
            -- cnqConquestInfoPrototype, etc -- ~370 in current corpus).
            -- Foundation for per-singleton decoders (#172 + future).
            CREATE TABLE IF NOT EXISTS singletons (
                fqn            TEXT PRIMARY KEY,
                payload_size   INTEGER NOT NULL,
                payload_b64    TEXT NOT NULL,
                string_count   INTEGER NOT NULL,
                cf_e0_count    INTEGER NOT NULL,
                cf_40_count    INTEGER NOT NULL,
                header_hex     TEXT NOT NULL,
                extracted_at   INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_singletons_size ON singletons(payload_size DESC);













        "#,
    )?;
    Ok(())
}
