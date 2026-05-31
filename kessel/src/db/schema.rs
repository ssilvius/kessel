//! Schema DDL + the `init_schema` table-creation step.
//!
//! `create_tables` currently holds the full DDL as one idempotent batch; the
//! per-domain `create_tables` split happens as each domain module is extracted
//! (it carves its CREATE statements out of here into its own module).

use anyhow::Result;
use rusqlite::Transaction;

/// Create every table and index (idempotent -- every statement is
/// `IF NOT EXISTS`). Runs inside the caller's transaction for atomicity.
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
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
                text TEXT NOT NULL,            -- Display text
                version INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_strings_locale ON strings(locale);
            CREATE INDEX IF NOT EXISTS idx_strings_id2 ON strings(id2);

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








            -- Schematic recipes (#60). Each itm.schem.* schematic has a
            -- companion schem.* GOM object whose payload encodes the recipe:
            -- output item GUID + material GUIDs with quantities. The schem.*
            -- companion is reachable via a CF GUID ref in the itm.schem.*
            -- payload. Output and materials are distinguished by the resolved
            -- FQN's prefix (itm.mat.* = material, anything else = output).
            CREATE TABLE IF NOT EXISTS schematics (
                schematic_fqn TEXT PRIMARY KEY,
                output_fqn TEXT,
                output_resolved INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS schematic_materials (
                schematic_fqn TEXT NOT NULL,
                material_fqn TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                PRIMARY KEY (schematic_fqn, material_fqn)
            );

            CREATE INDEX IF NOT EXISTS idx_schematic_materials_mat ON schematic_materials(material_fqn);

















            -- Conquest objectives: structured view of `ach.conquests.*` with
            -- category and cadence parsed from the FQN. After PR #38 these
            -- have working string_id resolution to names/descriptions.
            CREATE TABLE IF NOT EXISTS conquest_objectives (
                fqn         TEXT PRIMARY KEY,
                category    TEXT NOT NULL,   -- chapter|class|crafting|event|flashpoint|galactic_seasons|location|operation|spvp|uprisings|quest|weekly
                subcategory TEXT,            -- e.g. 'tatooine' (location), 'bounty' (event), 'bounty_hunter' (class)
                cadence     TEXT,            -- 'weekly' | 'daily' | NULL
                string_id   INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_conquest_objectives_category ON conquest_objectives(category);
            CREATE INDEX IF NOT EXISTS idx_conquest_objectives_subcategory ON conquest_objectives(subcategory);
            CREATE INDEX IF NOT EXISTS idx_conquest_objectives_cadence ON conquest_objectives(cadence);

            -- Conquest events from cnqConquestInfoPrototype singleton. One
            -- row per CF40 8C7DAFE5 record (90 events total). Each record:
            --   event_name (length-prefixed ASCII at marker tail)
            --   planet_code (CC 0BAC73FD + "_pla_<planet>" ASCII)
            --   record_size (bytes from this 8C7DAFE5 to the next one)
            --
            -- record_size is bimodal: single-planet invasion records cluster
            -- at 68-80 bytes; themed/special events (Yavin, Onderon, Iokath,
            -- Rishi, Ossus, CZ198, Ruhnuk, Meksha) cluster at 7700-8400 bytes
            -- with many inner sub-bonuses and CFE0 references. The two
            -- clusters are separated by a ~7400-byte empty gap. Observed
            -- split: 74 invasion / 16 themed.
            --
            -- CAVEAT: the FINAL record has no following marker, so its
            -- record_size is measured to end-of-payload and therefore absorbs
            -- the singleton's trailing top-level CFE0 reference array (~250
            -- extra bytes). event_kind is classified by a threshold placed
            -- inside the bimodal gap so this inflation cannot mis-tag the
            -- last event (see THEMED_SIZE_THRESHOLD).
            --
            -- Complements conquest_objectives (806 weekly tasks) by giving
            -- the actual event roster the tasks group under.
            CREATE TABLE IF NOT EXISTS conquest_events (
                ordinal      INTEGER PRIMARY KEY,
                event_name   TEXT NOT NULL,
                planet_code  TEXT,
                event_kind   TEXT NOT NULL,    -- 'invasion' | 'themed'
                record_size  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_conquest_events_name ON conquest_events(event_name);
            CREATE INDEX IF NOT EXISTS idx_conquest_events_planet ON conquest_events(planet_code);

            -- Weekly conquest rotation from the cnqSchedulePrototype singleton.
            -- 496 consecutive weekly entries: each maps a week_ordinal to the
            -- conquest event scheduled that week, resolved by matching the
            -- schedule's event GUID against the conquest event records in
            -- cnqConquestInfoPrototype (event_ordinal aligns with
            -- conquest_events.ordinal; event_name denormalized for convenience).
            --
            -- week_ordinal is a RELATIVE index (1001..1496), not a calendar
            -- date: the schedule carries no epoch anchor, so this is the
            -- rotation order. Absolute dates require pinning one known week
            -- downstream. event_ordinal/event_name are NULL when the
            -- scheduled GUID does not resolve to an event record.
            CREATE TABLE IF NOT EXISTS conquest_schedule (
                week_ordinal  INTEGER PRIMARY KEY,
                event_guid    TEXT NOT NULL,
                event_ordinal INTEGER,
                event_name    TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_conquest_schedule_event
                ON conquest_schedule(event_ordinal);

            -- Armor-class taxonomy from the cbtArmorTablePrototype singleton.
            -- One row per CF40 record: a small code byte and the class name.
            -- The canonical list of armor/equipment classes (medium,
            -- heavy_droid, focus, light, generator, heavy, shield_force,
            -- shield, adaptive). Self-validating via the names. The CFE0 refs
            -- in each record are short indexed refs, not 8-byte GUIDs, so they
            -- are intentionally not surfaced here.
            CREATE TABLE IF NOT EXISTS armor_classes (
                ordinal  INTEGER PRIMARY KEY,
                code     INTEGER NOT NULL,
                name     TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_armor_classes_name ON armor_classes(name);

            -- Raw combat stat-curve values from the cbtShieldPerLevel
            -- singleton. One row per typed float (0x04 tag + f32 LE) found
            -- inside the prototype's CF40 records, in payload order, grouped
            -- by the enclosing CF40 field-hash.
            --
            -- These are LITERAL stored values with NO semantic claim about
            -- level or stat: the curve is 2D (an undecoded segment key x
            -- level), so this table intentionally records (prototype,
            -- curve_hash, ordinal, value) only. Downstream (huttspawn) renders
            -- them as prose/charts; series separation is by curve_hash. The
            -- prototype column is retained so sibling per-level curves can be
            -- added later without a schema change.
            CREATE TABLE IF NOT EXISTS stat_curve_values (
                id          INTEGER PRIMARY KEY,
                prototype   TEXT NOT NULL,
                curve_hash  TEXT,
                ordinal     INTEGER NOT NULL,
                value       REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_stat_curve_values_proto
                ON stat_curve_values(prototype, curve_hash, ordinal);















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






























            -- Schematic typed columns (#140) -- 35 props from Schematic schema.
            CREATE TABLE IF NOT EXISTS schematic_details (
                fqn               TEXT PRIMARY KEY,
                profession        TEXT,
                tier              INTEGER,
                training_cost     INTEGER
            );



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
