//! SQLite database output with batched inserts

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::grammar::Grammar;
use crate::quest;
use crate::schema::item;
use crate::schema::GameObject;
use crate::stb::StbEntry;

/// Per-kind row counts inserted by `populate_conversation_refs`.
#[derive(Default, Debug)]
pub struct ConversationRefCounts {
    pub quest: u64,
    pub npc: u64,
    pub achievement: u64,
    pub codex: u64,
    pub item: u64,
    pub followup: u64,
    pub encounter: u64,
}

/// Serialized object ready for batch insert
struct PendingObject {
    guid: String,
    template_guid: String,
    fqn: String,
    game_id: String,
    kind: String,
    icon_name: Option<String>,
    string_id: Option<u32>,
    for_export: bool,
    version: u32,
    revision: u32,
    json: String,
}

/// Determine if an ability should be exported (not internal/debug)
fn should_export(fqn: &str) -> bool {
    // Internal/debug abilities to exclude
    const EXCLUDED_SLUGS: &[&str] = &[
        "exit_area",
        "quick_travel",
        "emergency_fleet_pass",
        "priority_transport",
        "heroic_moment",
        "legacy_",
        "mount_",
        "ooc_heal", // out of combat heal
        "ooc_regen",
        "rest",
        "revive",
        "holocom",
        "shuttle",
        "taxi",
        "speeder",
        "vehicle",
        "rocket_boost",
        "unity", // legacy ability
    ];

    let lower = fqn.to_lowercase();

    // Skip companion abilities entirely
    if lower.contains(".companion.") {
        return false;
    }

    // Check for excluded slugs
    for slug in EXCLUDED_SLUGS {
        if lower.contains(slug) {
            return false;
        }
    }

    true
}

/// Serialized string ready for batch insert
struct PendingString {
    fqn: String,
    locale: String,
    id1: u32,
    id2: u32,
    text: String,
    version: u32,
}

pub struct Database {
    conn: Mutex<Connection>,
    batch_size: usize,
    pending_objects: Mutex<Vec<PendingObject>>,
    pending_strings: Mutex<Vec<PendingString>>,
    grammar: Option<Arc<Grammar>>,
}

pub struct Stats {
    pub quests: u64,
    pub abilities: u64,
    pub items: u64,
    pub npcs: u64,
    pub strings: u64,
    pub chain_links: u64,
    pub npc_links: u64,
    pub reward_links: u64,
    pub runtime_ids: u64,
    pub missions: u64,
    pub conquest_objectives: u64,
    pub mission_npcs: u64,
    pub mission_rewards: u64,
    pub disciplines: u64,
    pub discipline_abilities: u64,
    pub talent_abilities: u64,
}

impl Database {
    pub fn with_grammar(path: &Path, grammar: Option<Arc<Grammar>>) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to create database")?;

        // Performance optimizations
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", "-64000")?; // 64MB cache
        conn.pragma_update(None, "temp_store", "MEMORY")?;

        Ok(Self {
            conn: Mutex::new(conn),
            batch_size: 5000,
            pending_objects: Mutex::new(Vec::with_capacity(5000)),
            pending_strings: Mutex::new(Vec::with_capacity(5000)),
            grammar,
        })
    }

    pub fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            -- Raw game objects table (everything we extract)
            CREATE TABLE IF NOT EXISTS objects (
                guid TEXT PRIMARY KEY,
                template_guid TEXT NOT NULL DEFAULT '',
                fqn TEXT NOT NULL,
                game_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                icon_name TEXT,
                string_id INTEGER,
                for_export INTEGER NOT NULL DEFAULT 1,
                version INTEGER NOT NULL DEFAULT 0,
                revision INTEGER NOT NULL DEFAULT 0,
                json TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_objects_fqn ON objects(fqn);
            CREATE INDEX IF NOT EXISTS idx_objects_game_id ON objects(game_id);
            CREATE INDEX IF NOT EXISTS idx_objects_kind ON objects(kind);
            CREATE INDEX IF NOT EXISTS idx_objects_for_export ON objects(for_export);
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
            CREATE VIEW IF NOT EXISTS quests AS
                SELECT * FROM objects WHERE kind = 'Quest';

            CREATE VIEW IF NOT EXISTS phases AS
                SELECT * FROM objects WHERE kind = 'Phase';

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
                SELECT * FROM objects WHERE kind = 'Ability' OR fqn LIKE 'abl.%';

            CREATE VIEW IF NOT EXISTS items AS
                SELECT * FROM objects WHERE kind = 'Item' OR fqn LIKE 'itm.%';

            CREATE VIEW IF NOT EXISTS npcs AS
                SELECT * FROM objects WHERE kind = 'Npc' OR fqn LIKE 'npc.%';

            -- Quest details (classified from FQN patterns)
            CREATE TABLE IF NOT EXISTS quest_details (
                fqn TEXT PRIMARY KEY,
                mission_type TEXT NOT NULL,
                faction TEXT,
                planet TEXT,
                class_code TEXT,
                companion_class TEXT,
                step_count INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_quest_details_type ON quest_details(mission_type);
            CREATE INDEX IF NOT EXISTS idx_quest_details_planet ON quest_details(planet);

            -- Item details (classified from FQN patterns; #59).
            -- Set name and set bonus require GOM payload parsing and are
            -- deferred to a follow-up issue.
            CREATE TABLE IF NOT EXISTS item_details (
                fqn TEXT PRIMARY KEY,
                item_kind TEXT NOT NULL,
                slot TEXT,
                weapon_type TEXT,
                armor_weight TEXT,
                rarity TEXT,
                item_level INTEGER,
                source TEXT,
                is_schematic INTEGER NOT NULL DEFAULT 0,
                crew_skill TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_item_details_kind ON item_details(item_kind);
            CREATE INDEX IF NOT EXISTS idx_item_details_slot ON item_details(slot);
            CREATE INDEX IF NOT EXISTS idx_item_details_source ON item_details(source);
            CREATE INDEX IF NOT EXISTS idx_item_details_rarity ON item_details(rarity);

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
            CREATE INDEX IF NOT EXISTS idx_item_details_crew_skill ON item_details(crew_skill);

            -- Conversation -> quest references. NODE conversation files (cnv.*)
            -- embed CF GUID refs to qst.* objects representing the quests
            -- that conversation grants or affects. ~23% of NODE files carry
            -- such refs in observed data. Populated by scanning .tor archives
            -- for NODE entries during the populate phase.
            CREATE TABLE IF NOT EXISTS conversation_quest_refs (
                cnv_fqn TEXT NOT NULL,
                quest_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, quest_fqn)
            );

            CREATE INDEX IF NOT EXISTS idx_cnv_quest_refs_quest ON conversation_quest_refs(quest_fqn);

            -- Conversation -> NPC actors. CF GUID refs in NODE bodies that
            -- match npc.* objects. NPC participants in the dialog (the cnv
            -- FQN's name segment usually picks out the primary NPC; this
            -- captures every actor present).
            CREATE TABLE IF NOT EXISTS conversation_npcs (
                cnv_fqn TEXT NOT NULL,
                npc_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, npc_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_npcs_npc ON conversation_npcs(npc_fqn);

            -- Conversation -> achievement unlocks. CF GUID refs to ach.*.
            CREATE TABLE IF NOT EXISTS conversation_achievements (
                cnv_fqn TEXT NOT NULL,
                achievement_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, achievement_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_ach_ach ON conversation_achievements(achievement_fqn);

            -- Conversation -> codex unlocks. CF GUID refs to cdx.*.
            CREATE TABLE IF NOT EXISTS conversation_codex (
                cnv_fqn TEXT NOT NULL,
                codex_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, codex_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_cdx_cdx ON conversation_codex(codex_fqn);

            -- Conversation -> item grants. CF GUID refs to itm.* (rewards
            -- mailed/awarded by the dialog).
            CREATE TABLE IF NOT EXISTS conversation_items (
                cnv_fqn TEXT NOT NULL,
                item_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, item_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_items_item ON conversation_items(item_fqn);

            -- Conversation -> follow-up conversation. CF GUID refs to other
            -- cnv.* objects (sequel dialogs, branching outcomes).
            CREATE TABLE IF NOT EXISTS conversation_followups (
                cnv_fqn TEXT NOT NULL,
                target_cnv_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, target_cnv_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_follow_target ON conversation_followups(target_cnv_fqn);

            -- Conversation -> combat encounter. CF GUID refs to enc.* (combat
            -- triggered by the dialog).
            CREATE TABLE IF NOT EXISTS conversation_encounters (
                cnv_fqn TEXT NOT NULL,
                encounter_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, encounter_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_enc_enc ON conversation_encounters(encounter_fqn);

            -- Quest NPC references (npc.* FQNs embedded in payload)
            CREATE TABLE IF NOT EXISTS quest_npcs (
                quest_fqn TEXT NOT NULL,
                npc_fqn TEXT NOT NULL,
                PRIMARY KEY (quest_fqn, npc_fqn)
            );

            -- Quest phase references (mpn.* FQNs embedded in payload)
            CREATE TABLE IF NOT EXISTS quest_phases (
                quest_fqn TEXT NOT NULL,
                phase_fqn TEXT NOT NULL,
                PRIMARY KEY (quest_fqn, phase_fqn)
            );

            -- Quest prerequisites (has_* variables in payload)
            CREATE TABLE IF NOT EXISTS quest_prerequisites (
                fqn TEXT NOT NULL,
                variable TEXT NOT NULL,
                PRIMARY KEY (fqn, variable)
            );

            -- Quest chain links (built from GUID refs and prereq graph)
            -- Uses game_id (sha256(fqn:guid)[0:16]) not FQN, since FQN is
            -- not unique in the objects table (guid is the true PK)
            CREATE TABLE IF NOT EXISTS quest_chain (
                source_game_id TEXT NOT NULL,
                target_game_id TEXT NOT NULL,
                link_type TEXT NOT NULL,
                PRIMARY KEY (source_game_id, target_game_id)
            );

            -- Missions: unified mission identities from two sources.
            --
            -- 1. Every qst.* object is a mission (source='qst').
            -- 2. Every unique mpn-prefix grouping (a path-prefix of mpn.* objects
            --    formed by dropping the leaf segment) that has no qst.* parent
            --    is also a mission (source='mpn-prefix'). These are typically
            --    alliance alerts, side missions encoded purely as phase trees,
            --    and other content that lives only as mpn.* phases.
            --
            -- Closes the 3.9k vs 1.3k gap from #34.
            CREATE TABLE IF NOT EXISTS missions (
                mission_fqn TEXT PRIMARY KEY,
                source      TEXT NOT NULL  -- 'qst' or 'mpn-prefix'
            );

            CREATE INDEX IF NOT EXISTS idx_missions_source ON missions(source);

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

            -- Mission NPCs: NPC references aggregated across a mission's
            -- entire phase tree. For qst-source missions this is the quest's
            -- own NPCs (same as quest_npcs). For mpn-prefix missions (alliance
            -- alerts, mpn-only side missions) this aggregates NPCs from every
            -- mpn.<prefix>.* child phase. Closes the gap where quest_npcs
            -- only saw qst.* objects -- mission_npcs sees the full mission.
            CREATE TABLE IF NOT EXISTS mission_npcs (
                mission_fqn TEXT NOT NULL,
                npc_fqn     TEXT NOT NULL,
                PRIMARY KEY (mission_fqn, npc_fqn)
            );

            CREATE INDEX IF NOT EXISTS idx_mission_npcs_npc ON mission_npcs(npc_fqn);

            -- Mission rewards: same idea -- quest_reward_* variable names
            -- aggregated across the mission's entire phase tree.
            CREATE TABLE IF NOT EXISTS mission_rewards (
                mission_fqn     TEXT NOT NULL,
                reward_variable TEXT NOT NULL,
                PRIMARY KEY (mission_fqn, reward_variable)
            );

            CREATE INDEX IF NOT EXISTS idx_mission_rewards_variable ON mission_rewards(reward_variable);

            -- Spawn runtime IDs: every SPN triple `spn.X;target.Y;<id>` in a
            -- quest payload becomes one row. The numeric ID may be the runtime
            -- node ID the combat log emits when the entity is interacted with
            -- (hypothesis from #20, awaiting log verification). Even if it
            -- turns out to be packed coordinates, the bridge data lives here.
            CREATE TABLE IF NOT EXISTS spawn_runtime_ids (
                spn_fqn     TEXT NOT NULL,
                target_fqn  TEXT NOT NULL,
                runtime_id  INTEGER NOT NULL,
                PRIMARY KEY (spn_fqn, target_fqn, runtime_id)
            );

            CREATE INDEX IF NOT EXISTS idx_spawn_runtime_ids_target ON spawn_runtime_ids(target_fqn);
            CREATE INDEX IF NOT EXISTS idx_spawn_runtime_ids_runtime ON spawn_runtime_ids(runtime_id);

            -- Quest rewards (variable names extracted from payloads, e.g.
            -- 'quest_reward_adrenal'). Variable names are categories
            -- (adrenal, medpac, alignment) -- specific items are engine-
            -- resolved at runtime and not in payload data.
            CREATE TABLE IF NOT EXISTS quest_rewards (
                quest_fqn       TEXT NOT NULL,
                reward_variable TEXT NOT NULL,
                PRIMARY KEY (quest_fqn, reward_variable)
            );

            CREATE INDEX IF NOT EXISTS idx_quest_rewards_variable ON quest_rewards(reward_variable);

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
                WHERE o.fqn LIKE 'mpn.%.bonus.%';

            -- Disciplines: one row per (class, discipline) pair derived from
            -- abl.{class}.skill.{discipline}.* FQN patterns.
            CREATE TABLE IF NOT EXISTS disciplines (
                class_code       TEXT NOT NULL,
                discipline_name  TEXT NOT NULL,
                fqn_prefix       TEXT NOT NULL,  -- e.g. "abl.jedi_knight.skill.defense"
                PRIMARY KEY (class_code, discipline_name)
            );

            CREATE INDEX IF NOT EXISTS idx_disciplines_class ON disciplines(class_code);

            -- Discipline abilities: every abl.* that belongs to a discipline,
            -- with tier level and slot type derived from FQN segments.
            -- tier_level: NULL for base abilities (no mods segment), else
            --   15/23/27/35/39/43/47/51/60/64/68/73/78 from tal.* payload.
            -- slot_type: 'core' | 'choice' | 'utility' | 'special' | 'passive' | 'base'
            CREATE TABLE IF NOT EXISTS discipline_abilities (
                discipline_fqn_prefix  TEXT NOT NULL,
                ability_game_id        TEXT NOT NULL,
                ability_fqn            TEXT NOT NULL,
                tier_level             INTEGER,
                slot_type              TEXT NOT NULL,
                PRIMARY KEY (discipline_fqn_prefix, ability_game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_discipline_abilities_disc ON discipline_abilities(discipline_fqn_prefix);
            CREATE INDEX IF NOT EXISTS idx_discipline_abilities_abl  ON discipline_abilities(ability_game_id);

            -- Talent → ability links: GUID refs decoded from tal.* payloads.
            -- 37% of talents reference 1-3 abilities via CC 17E2840B + CF GUID pattern.
            CREATE TABLE IF NOT EXISTS talent_abilities (
                talent_game_id   TEXT NOT NULL,
                talent_fqn       TEXT NOT NULL,
                ability_game_id  TEXT NOT NULL,
                ability_fqn      TEXT,           -- NULL if GUID not in our object set
                PRIMARY KEY (talent_game_id, ability_game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_talent_abilities_talent  ON talent_abilities(talent_game_id);
            CREATE INDEX IF NOT EXISTS idx_talent_abilities_ability ON talent_abilities(ability_game_id);

            -- Extraction metadata
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;

        Ok(())
    }

    /// Queue an object for batch insert
    pub fn insert_object(&self, obj: &GameObject) -> Result<()> {
        if obj.guid.is_empty() {
            return Ok(()); // Skip objects without GUID
        }

        let json_str = serde_json::to_string(&obj.json)?;
        let pending = PendingObject {
            guid: obj.guid.clone(),
            template_guid: obj.template_guid.clone(),
            fqn: obj.fqn.clone(),
            game_id: obj.game_id.clone(),
            kind: obj.kind.clone(),
            icon_name: obj.icon_name.clone(),
            string_id: obj.string_id,
            for_export: should_export(&obj.fqn),
            version: obj.version,
            revision: obj.revision,
            json: json_str,
        };

        let mut objects = self.pending_objects.lock().unwrap();
        objects.push(pending);

        if objects.len() >= self.batch_size {
            let batch: Vec<_> = objects.drain(..).collect();
            drop(objects); // Release lock before flushing
            self.flush_objects(batch)?;
        }

        Ok(())
    }

    /// Queue a string for batch insert
    /// If grammar rules are configured, applies them to clean the text
    pub fn insert_string(&self, fqn: &str, locale: &str, entry: &StbEntry) -> Result<()> {
        // Apply grammar rules if configured
        let cleaned_text = if let Some(ref grammar) = self.grammar {
            grammar.clean(&entry.text)
        } else {
            entry.text.clone()
        };

        let pending = PendingString {
            fqn: fqn.to_string(),
            locale: locale.to_string(),
            id1: entry.id1,
            id2: entry.id2,
            text: cleaned_text,
            version: entry.version,
        };

        let mut strings = self.pending_strings.lock().unwrap();
        strings.push(pending);

        if strings.len() >= self.batch_size {
            let batch: Vec<_> = strings.drain(..).collect();
            drop(strings); // Release lock before flushing
            self.flush_strings(batch)?;
        }

        Ok(())
    }

    /// Flush pending objects to database in a single transaction
    fn flush_objects(&self, batch: Vec<PendingObject>) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                r#"
                INSERT INTO objects (guid, template_guid, fqn, game_id, kind, icon_name, string_id, for_export, version, revision, json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(guid) DO UPDATE SET
                    template_guid = excluded.template_guid,
                    fqn = excluded.fqn,
                    game_id = excluded.game_id,
                    kind = excluded.kind,
                    icon_name = excluded.icon_name,
                    string_id = excluded.string_id,
                    for_export = excluded.for_export,
                    version = excluded.version,
                    revision = excluded.revision,
                    json = excluded.json
                WHERE excluded.revision > objects.revision
                "#,
            )?;

            for obj in &batch {
                stmt.execute(params![
                    obj.guid,
                    obj.template_guid,
                    obj.fqn,
                    obj.game_id,
                    obj.kind,
                    obj.icon_name,
                    obj.string_id,
                    obj.for_export,
                    obj.version,
                    obj.revision,
                    obj.json
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Flush pending strings to database in a single transaction
    fn flush_strings(&self, batch: Vec<PendingString>) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                r#"
                INSERT INTO strings (fqn, locale, id1, id2, text, version)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(fqn) DO UPDATE SET
                    locale = excluded.locale,
                    id1 = excluded.id1,
                    id2 = excluded.id2,
                    text = excluded.text,
                    version = excluded.version
                WHERE excluded.version > strings.version
                "#,
            )?;

            for s in &batch {
                stmt.execute(params![s.fqn, s.locale, s.id1, s.id2, s.text, s.version])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Flush any remaining pending inserts
    pub fn flush(&self) -> Result<()> {
        // Flush objects
        let objects: Vec<_> = {
            let mut pending = self.pending_objects.lock().unwrap();
            pending.drain(..).collect()
        };
        self.flush_objects(objects)?;

        // Flush strings
        let strings: Vec<_> = {
            let mut pending = self.pending_strings.lock().unwrap();
            pending.drain(..).collect()
        };
        self.flush_strings(strings)?;

        Ok(())
    }

    /// Populate quest tables from extracted objects (second pass).
    ///
    /// Reads all quest objects, classifies them by FQN, and extracts embedded
    /// references (NPCs, phases, prerequisites) from the base64 payload.
    /// Must be called after all objects and strings are flushed.
    /// Collapse multi-GUID FQN rows down to one "best" row per FQN.
    ///
    /// During extraction, the same FQN can appear under multiple GUIDs --
    /// canonical objects with full payload, plus stub references that share
    /// the FQN. The in-memory accept_variant filter blocks inferior variants
    /// that follow a superior one, but cannot retroactively remove a stub
    /// that was inserted before the canonical version arrived. This pass
    /// keeps the row with the highest "extraction quality" per FQN: prefers
    /// non-NULL string_id, then non-NULL icon_name, then larger json payload.
    pub fn dedup_objects_by_fqn(&self) -> Result<u64> {
        self.flush()?;
        let conn = self.conn.lock().unwrap();
        let before: u64 = conn.query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0))?;

        conn.execute(
            r#"
            DELETE FROM objects WHERE rowid IN (
                SELECT rowid FROM (
                    SELECT rowid, ROW_NUMBER() OVER (
                        PARTITION BY fqn
                        ORDER BY (string_id IS NOT NULL) DESC,
                                 (icon_name IS NOT NULL) DESC,
                                 length(json) DESC,
                                 guid ASC
                    ) AS rn FROM objects
                ) WHERE rn > 1
            )
            "#,
            [],
        )?;

        let after: u64 = conn.query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0))?;
        Ok(before - after)
    }

    pub fn populate_quest_tables(&self) -> Result<u64> {
        self.flush()?;

        // Read phase: load names and quest objects into memory
        let (name_cache, rows) = {
            let conn = self.conn.lock().unwrap();

            let mut name_cache: std::collections::HashMap<u32, String> =
                std::collections::HashMap::new();
            {
                let mut stmt = conn.prepare("SELECT id2, text FROM strings WHERE id1 = 88")?;
                let name_rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in name_rows {
                    let (id2, text) = row?;
                    name_cache.insert(id2, text);
                }
            }

            let mut stmt =
                conn.prepare("SELECT fqn, string_id, json FROM objects WHERE kind = 'Quest'")?;
            let rows: Vec<(String, Option<u32>, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<u32>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            (name_cache, rows)
        };

        // Write phase: classify and insert into quest tables
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut detail_count = 0u64;

        {
            let mut detail_stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO quest_details (fqn, mission_type, faction, planet, class_code, companion_class, step_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut npc_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO quest_npcs (quest_fqn, npc_fqn) VALUES (?1, ?2)",
            )?;
            let mut phase_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO quest_phases (quest_fqn, phase_fqn) VALUES (?1, ?2)",
            )?;
            let mut prereq_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO quest_prerequisites (fqn, variable) VALUES (?1, ?2)",
            )?;

            for (fqn, string_id, json_str) in &rows {
                // Get quest name for classification overrides
                let name = string_id
                    .and_then(|sid| name_cache.get(&sid))
                    .map(|s| s.as_str())
                    .unwrap_or("");

                let details = quest::classify(fqn, name);

                // Count steps from payload strings (branch/step/task patterns)
                let step_count = count_quest_steps(json_str);

                detail_stmt.execute(params![
                    details.fqn,
                    details.mission_type,
                    details.faction,
                    details.planet,
                    details.class_code,
                    details.companion_class,
                    step_count,
                ])?;
                detail_count += 1;

                // Extract embedded FQN references from payload strings
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(strings) = json.get("strings").and_then(|s| s.as_array()) {
                        for s in strings {
                            if let Some(ref_str) = s.as_str() {
                                if ref_str.starts_with("npc.") {
                                    npc_stmt.execute(params![fqn, ref_str])?;
                                } else if ref_str.starts_with("mpn.") {
                                    phase_stmt.execute(params![fqn, ref_str])?;
                                } else if ref_str.starts_with("has_") {
                                    prereq_stmt.execute(params![fqn, ref_str])?;
                                }
                            }
                        }
                    }
                }
            }
        }

        tx.commit()?;
        Ok(detail_count)
    }

    /// Populate `item_details` from every `kind = 'Item'` row by classifying
    /// the FQN. Mirrors `populate_quest_tables` in shape.
    pub fn populate_item_tables(&self) -> Result<u64> {
        self.flush()?;

        let rows: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT fqn FROM objects WHERE kind = 'Item'")?;
            let collected: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0u64;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO item_details (fqn, item_kind, slot, weapon_type, armor_weight, rarity, item_level, source, is_schematic, crew_skill) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for fqn in &rows {
                let d = item::classify(fqn);
                stmt.execute(params![
                    d.fqn,
                    d.item_kind,
                    d.slot,
                    d.weapon_type,
                    d.armor_weight,
                    d.rarity,
                    d.item_level,
                    d.source,
                    if d.is_schematic { 1 } else { 0 },
                    d.crew_skill,
                ])?;
                count += 1;
            }
        }

        tx.commit()?;
        Ok(count)
    }

    /// Populate `quest_chain` by scanning every quest payload for `0xCF` type
    /// markers followed by 8 bytes that decode as a big-endian GUID belonging
    /// to another quest object.
    ///
    /// The previous attempt (PR #11, removed in #19) read the 8 bytes as
    /// little-endian and found zero matches. GUIDs in SWTOR payloads are stored
    /// big-endian; flipping to BE produces real chain links (e.g. broken_blades
    /// -> breaking_the_blades bonus, revanites_revealed -> intro_rishii_village).
    pub fn populate_quest_chain(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();

        // Build a map of GUID (uppercase hex) -> game_id for all quest objects.
        let mut guid_to_game_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt =
                conn.prepare("SELECT guid, game_id FROM objects WHERE fqn LIKE 'qst.%'")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows.filter_map(|r| r.ok()) {
                guid_to_game_id.insert(row.0.to_uppercase(), row.1);
            }
        }

        let payloads = {
            let mut stmt = conn.prepare(
                "SELECT guid, game_id, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE fqn LIKE 'qst.%'",
            )?;
            let rows: Vec<(String, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let tx = conn.unchecked_transaction()?;
        let mut count: u64 = 0;

        for (src_guid, src_game_id, payload_b64) in &payloads {
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
            let payload = match BASE64.decode(payload_b64) {
                Ok(b) => b,
                Err(_) => continue,
            };

            let mut i = 0;
            while i + 9 <= payload.len() {
                if payload[i] == 0xCF {
                    // 8 bytes big-endian GUID
                    let ref_guid = payload[i + 1..i + 9]
                        .iter()
                        .map(|b| format!("{:02X}", b))
                        .collect::<String>();

                    if ref_guid != src_guid.to_uppercase() {
                        if let Some(target_game_id) = guid_to_game_id.get(&ref_guid) {
                            tx.execute(
                                "INSERT OR IGNORE INTO quest_chain \
                                 (source_game_id, target_game_id, link_type) \
                                 VALUES (?1, ?2, 'guid_ref')",
                                params![src_game_id, target_game_id],
                            )?;
                            count += 1;
                        }
                    }
                    i += 9;
                } else {
                    i += 1;
                }
            }
        }

        tx.commit()?;
        Ok(count)
    }

    /// Populate `schematics` and `schematic_materials` from `itm.schem.*` +
    /// `schem.*` payloads.
    ///
    /// Each `itm.schem.*` object's payload carries a CF GUID ref to a
    /// companion `schem.*` object (different GOM kind, ~14k instances). The
    /// schem.* payload encodes the recipe: a list of CF GUID refs each
    /// followed by a quantity byte. Resolved FQNs are split by prefix:
    /// `itm.mat.*` rows go to `schematic_materials`, anything else is treated
    /// as the output and stored in `schematics.output_fqn`.
    ///
    /// The quantity byte sits immediately after each 9-byte CF marker
    /// (`CF E0 NN NN NN NN NN NN NN`). Material values run 1-99 in observed
    /// payloads (low-bit-set non-CF bytes); the parser clamps to 0..99 to
    /// reject obviously-non-quantity bytes.
    pub fn populate_schematic_recipes(&self) -> Result<u64> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use std::collections::HashMap;

        let conn = self.conn.lock().unwrap();

        // Build GUID -> FQN map for all objects (only need one lookup table).
        let mut guid_to_fqn: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT guid, fqn FROM objects")?;
            for row in stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
            {
                guid_to_fqn.insert(row.0.to_uppercase(), row.1);
            }
        }

        // Map itm.schem.<X> -> schem.<X> via the strip-prefix convention,
        // resolved by FQN match (cheap and reliable; the CF ref out of the
        // itm.schem.* payload would also work but adds a dump pass).
        // Build schem.* fqn -> payload_b64 map (single scan, indexed lookup).
        let schem_payloads: HashMap<String, String> = {
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'schem'",
            )?;
            let collected: HashMap<String, String> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        // Pair each itm.schem.* with its schem.* companion via the strip-prefix
        // convention. In-memory map lookup avoids the quadratic SQL JOIN that
        // would otherwise run REPLACE() against every row pair.
        let itm_to_schem: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT fqn FROM objects WHERE fqn LIKE 'itm.schem.%' AND kind = 'Item'",
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|itm_fqn| {
                    let schem_fqn = itm_fqn.replacen("itm.schem.", "schem.", 1);
                    schem_payloads.get(&schem_fqn).map(|p| (itm_fqn, p.clone()))
                })
                .collect();
            collected
        };

        let tx = conn.unchecked_transaction()?;
        let mut schem_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO schematics (schematic_fqn, output_fqn, output_resolved) \
             VALUES (?1, ?2, ?3)",
        )?;
        let mut mat_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO schematic_materials (schematic_fqn, material_fqn, quantity) \
             VALUES (?1, ?2, ?3)",
        )?;

        let mut count = 0u64;
        for (schematic_fqn, payload_b64) in &itm_to_schem {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };

            let mut output_fqn: Option<String> = None;
            let mut materials: Vec<(String, u32)> = Vec::new();

            let mut i = 0;
            while i + 10 <= payload.len() {
                if payload[i] == 0xCF && payload[i + 1] == 0xE0 {
                    let ref_guid: String = payload[i + 1..i + 9]
                        .iter()
                        .map(|b| format!("{:02X}", b))
                        .collect();
                    let qty_byte = payload[i + 9];
                    if let Some(fqn) = guid_to_fqn.get(&ref_guid) {
                        if fqn.starts_with("itm.mat.") {
                            // Quantity follows the 9-byte CF marker. Reject
                            // values >99 to avoid mistaking a continuation
                            // byte for a quantity.
                            let qty = if qty_byte == 0 || qty_byte > 99 {
                                1
                            } else {
                                qty_byte as u32
                            };
                            materials.push((fqn.clone(), qty));
                        } else if fqn.starts_with("itm.")
                            && !fqn.starts_with("itm.schem.")
                            && fqn != schematic_fqn
                            && output_fqn.is_none()
                        {
                            output_fqn = Some(fqn.clone());
                        }
                    }
                    i += 9;
                } else {
                    i += 1;
                }
            }

            let resolved = output_fqn.is_some() as i32;
            schem_stmt.execute(params![schematic_fqn, output_fqn, resolved])?;
            count += 1;
            for (mat_fqn, qty) in &materials {
                mat_stmt.execute(params![schematic_fqn, mat_fqn, qty])?;
            }
        }

        drop(schem_stmt);
        drop(mat_stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `conversation_quest_refs` by scanning every NODE prototype
    /// file in `tor_dir` for CF GUID refs that resolve to a known quest.
    ///
    /// NODE files at `/resources/systemgenerated/prototypes/<num>.node` hold
    /// the full conversation playback data for `cnv.*` objects. The PROT
    /// header (bytes 0x14..) carries the cnv FQN. The body contains CF E0
    /// GUID refs; those that match a quest GUID indicate the conversation
    /// grants or otherwise affects that quest. Empirically ~23% of NODE
    /// files carry such refs.
    pub fn populate_conversation_refs(
        &self,
        tor_dir: &std::path::Path,
        hashes: &crate::hash::HashDictionary,
    ) -> Result<ConversationRefCounts> {
        use crate::myp::Archive;
        use std::collections::{HashMap, HashSet};

        let conn = self.conn.lock().unwrap();

        // Build a single GUID -> (kind, fqn) map for all objects, so a single
        // CF E0 scan resolves to its target without per-kind lookups.
        let guid_to_kind_fqn: HashMap<[u8; 8], (String, String)> = {
            let mut stmt = conn.prepare("SELECT guid, kind, fqn FROM objects")?;
            let collected: HashMap<[u8; 8], (String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .filter_map(|(guid_hex, kind, fqn)| {
                    if guid_hex.len() != 16 {
                        return None;
                    }
                    let mut bytes = [0u8; 8];
                    for i in 0..8 {
                        bytes[i] = u8::from_str_radix(&guid_hex[i * 2..i * 2 + 2], 16).ok()?;
                    }
                    Some((bytes, (kind, fqn)))
                })
                .collect();
            collected
        };

        let prototype_hashes: HashSet<u64> = hashes
            .paths_matching("/resources/systemgenerated/prototypes/")
            .into_iter()
            .map(|(h, _)| h)
            .collect();

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let tx = conn.unchecked_transaction()?;
        let mut quest_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_quest_refs (cnv_fqn, quest_fqn) VALUES (?1, ?2)",
        )?;
        let mut npc_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_npcs (cnv_fqn, npc_fqn) VALUES (?1, ?2)",
        )?;
        let mut ach_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_achievements (cnv_fqn, achievement_fqn) VALUES (?1, ?2)",
        )?;
        let mut cdx_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_codex (cnv_fqn, codex_fqn) VALUES (?1, ?2)",
        )?;
        let mut item_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_items (cnv_fqn, item_fqn) VALUES (?1, ?2)",
        )?;
        let mut follow_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_followups (cnv_fqn, target_cnv_fqn) VALUES (?1, ?2)",
        )?;
        let mut enc_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO conversation_encounters (cnv_fqn, encounter_fqn) VALUES (?1, ?2)",
        )?;

        let mut counts = ConversationRefCounts::default();

        for tor_path in &tor_files {
            let mut archive = match Archive::open(tor_path) {
                Ok(a) => a,
                Err(_) => continue,
            };
            let entries: Vec<_> = match archive.entries() {
                Ok(e) => e.cloned().collect(),
                Err(_) => continue,
            };
            for entry in &entries {
                if !prototype_hashes.contains(&entry.filename_hash) {
                    continue;
                }
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let fqn_start = 0x14;
                if data.len() < fqn_start + 8 {
                    continue;
                }
                let mut fqn_end = fqn_start;
                while fqn_end < data.len() && fqn_end < fqn_start + 200 && data[fqn_end] != 0 {
                    fqn_end += 1;
                }
                let cnv_fqn = String::from_utf8_lossy(&data[fqn_start..fqn_end]).to_string();
                if !cnv_fqn.starts_with("cnv.") {
                    continue;
                }

                // Per-target dedup: a single conversation often references the
                // same target multiple times (one per dialog branch); collapse.
                let mut seen: HashSet<&str> = HashSet::new();
                let mut i = 0;
                while i + 9 <= data.len() {
                    if data[i] == 0xCF && data[i + 1] == 0xE0 {
                        let mut g = [0u8; 8];
                        g.copy_from_slice(&data[i + 1..i + 9]);
                        if let Some((kind, target_fqn)) = guid_to_kind_fqn.get(&g) {
                            if seen.insert(target_fqn.as_str()) {
                                match kind.as_str() {
                                    "Quest" => {
                                        quest_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.quest += 1;
                                    }
                                    "Npc" => {
                                        npc_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.npc += 1;
                                    }
                                    "Achievement" => {
                                        ach_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.achievement += 1;
                                    }
                                    "Codex" => {
                                        cdx_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.codex += 1;
                                    }
                                    "Item" => {
                                        item_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.item += 1;
                                    }
                                    "Conversation" if target_fqn != &cnv_fqn => {
                                        follow_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.followup += 1;
                                    }
                                    "Encounter" => {
                                        enc_stmt.execute(params![cnv_fqn, target_fqn])?;
                                        counts.encounter += 1;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        i += 9;
                    } else {
                        i += 1;
                    }
                }
            }
        }

        drop(quest_stmt);
        drop(npc_stmt);
        drop(ach_stmt);
        drop(cdx_stmt);
        drop(item_stmt);
        drop(follow_stmt);
        drop(enc_stmt);
        tx.commit()?;
        Ok(counts)
    }

    /// Populate `quest_chain` with FQN-derived arc-ordering edges.
    ///
    /// SWTOR quest payloads do not carry direct GUID refs for story-arc
    /// progression -- but the FQN segments do. Two patterns encode order:
    ///
    /// 1. Class-story act bridges:
    ///    `qst.location.open_world.<faction>.act_<N>.<class>.<quest>` --
    ///    every quest at act_N within the same (faction, class) bucket
    ///    must be done before unlocking act_(N+1). Edge per A in act_N to
    ///    every B in act_(N+1).
    ///
    /// 2. Expansion world-arc hub bridges:
    ///    `qst.exp.<NN>.<planet>.world_arc.<faction>.hub_<N>.<quest>` --
    ///    every quest at hub_N within the same (exp, planet, faction)
    ///    bucket must be done before unlocking hub_(N+1). Edge per A in
    ///    hub_N to every B in hub_(N+1).
    ///
    /// `bonus.*` and `temp_*_prereq` placeholder quests are filtered out --
    /// bonuses already attach via `guid_ref`, prereq placeholders are
    /// internal artifacts not real story content.
    ///
    /// Edges land with `link_type='fqn_arc_order'` so consumers can filter
    /// derived from real GUID-ref edges.
    pub fn populate_quest_chain_fqn_order(&self) -> Result<u64> {
        use std::collections::HashMap;

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT fqn, game_id FROM objects WHERE kind = 'Quest'")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        // bucket_key -> position -> Vec<game_id>
        let mut buckets: HashMap<String, HashMap<u32, Vec<String>>> = HashMap::new();

        for (fqn, game_id) in &rows {
            if fqn.contains(".bonus.") || fqn.contains(".temp_") {
                continue;
            }
            let parts: Vec<&str> = fqn.split('.').collect();

            // Pattern 1: qst.location.open_world.<faction>.act_<N>.<class>.<quest>
            if parts.len() >= 7
                && parts[0] == "qst"
                && parts[1] == "location"
                && parts[2] == "open_world"
            {
                let faction = parts[3];
                if let Some(n) = parts[4]
                    .strip_prefix("act_")
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    let class = parts[5];
                    let key = format!("act|{}|{}", faction, class);
                    buckets
                        .entry(key)
                        .or_default()
                        .entry(n)
                        .or_default()
                        .push(game_id.clone());
                    continue;
                }
            }

            // Pattern 2: qst.exp.<NN>.<planet>.world_arc.<faction>.hub_<N>.<quest>
            if parts.len() >= 8 && parts[0] == "qst" && parts[1] == "exp" && parts[4] == "world_arc"
            {
                let exp = parts[2];
                let planet = parts[3];
                let faction = parts[5];
                if let Some(n) = parts[6]
                    .strip_prefix("hub_")
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    let key = format!("hub|{}|{}|{}", exp, planet, faction);
                    buckets
                        .entry(key)
                        .or_default()
                        .entry(n)
                        .or_default()
                        .push(game_id.clone());
                }
            }
        }

        let tx = conn.unchecked_transaction()?;
        let mut count = 0u64;
        for positions in buckets.values() {
            let mut keys: Vec<&u32> = positions.keys().collect();
            keys.sort();
            for window in keys.windows(2) {
                let (lo, hi) = (window[0], window[1]);
                let sources = &positions[lo];
                let targets = &positions[hi];
                for src in sources {
                    for tgt in targets {
                        tx.execute(
                            "INSERT OR IGNORE INTO quest_chain \
                             (source_game_id, target_game_id, link_type) \
                             VALUES (?1, ?2, 'fqn_arc_order')",
                            params![src, tgt],
                        )?;
                        count += 1;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// Populate `quest_chain` with `planet_transition` links by scanning every
    /// `leaving_{planet}` quest for strings that name the destination.
    ///
    /// Pattern: strings containing `_to_{planet}` (e.g. `jrn_start_take_the_shuttle_to_dromund_kaas`)
    /// are used to locate the class intro quest at that planet. Strings that name
    /// intermediate stops (e.g. `the_imperial_transit_station`) produce no match
    /// and are silently skipped.
    pub fn populate_planet_transitions(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();

        // Build lookup: fqn -> game_id for all intro quests.
        let mut intro_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT fqn, game_id FROM objects WHERE fqn LIKE 'qst.location.%.class.%.intro'",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows.filter_map(|r| r.ok()) {
                intro_map.insert(row.0, row.1);
            }
        }

        let mut leaving_quests: Vec<(String, String, String)> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT fqn, game_id, json_extract(json, '$.strings') \
                 FROM objects \
                 WHERE fqn LIKE 'qst.location.%.class.%.leaving_%' \
                   AND json_extract(json, '$.strings') IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows.filter_map(|r| r.ok()) {
                leaving_quests.push(row);
            }
        }

        let tx = conn.unchecked_transaction()?;
        let mut count: u64 = 0;

        for (fqn, game_id, strings_json) in &leaving_quests {
            // Extract class segment: qst.location.{planet}.class.{class}.leaving_{planet}
            let parts: Vec<&str> = fqn.split('.').collect();
            let class_pos = parts.iter().position(|&p| p == "class");
            let class = match class_pos {
                Some(i) if i + 1 < parts.len() => parts[i + 1],
                _ => continue,
            };

            let strings: Vec<String> = match serde_json::from_str(strings_json) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Scan strings for `_to_{dest}` patterns; try each as a planet FQN component.
            for s in &strings {
                if let Some(dest) = extract_transit_dest(s) {
                    let intro_fqn = format!("qst.location.{}.class.{}.intro", dest, class);
                    if let Some(target_game_id) = intro_map.get(&intro_fqn) {
                        tx.execute(
                            "INSERT OR IGNORE INTO quest_chain \
                             (source_game_id, target_game_id, link_type) \
                             VALUES (?1, ?2, 'planet_transition')",
                            params![game_id, target_game_id],
                        )?;
                        count += 1;
                        break;
                    }
                }
            }
        }

        tx.commit()?;
        Ok(count)
    }
}

/// Extract the destination planet component from a transit tracking/journal string.
///
/// Matches strings containing `_to_{dest}` where `{dest}` consists of lowercase
/// letters and underscores. Strips a leading `the_` if present. The caller filters
/// by checking for a matching intro quest, so non-planet results (e.g. `imperial_transit_station`)
/// are silently dropped.
fn extract_transit_dest(s: &str) -> Option<String> {
    let idx = s.find("_to_")?;
    let after = &s[idx + 4..];
    let dest = after.strip_prefix("the_").unwrap_or(after);
    if !dest.is_empty() && dest.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        Some(dest.to_string())
    } else {
        None
    }
}

/// Parse a conquest objective FQN (`ach.conquests.<category>.<sub>...<leaf>`)
/// into (category, subcategory, cadence) where cadence is one of:
///   - `Some("weekly")` if the leaf ends with `_weekly` or path contains `.weekly.`
///   - `Some("daily")` if the path contains `.daily.`
///   - `None` for repeatable / any-cadence objectives
fn parse_conquest_fqn(fqn: &str) -> (String, Option<String>, Option<String>) {
    // Expected shape: ach.conquests.<category>[.<subcategory>][...].<leaf>
    let parts: Vec<&str> = fqn.split('.').collect();
    if parts.len() < 4 || parts[0] != "ach" || parts[1] != "conquests" {
        return ("unknown".to_string(), None, None);
    }
    let category = parts[2].to_string();
    let subcategory = if parts.len() >= 5 {
        Some(parts[3].to_string())
    } else {
        None
    };

    // Cadence: leaf-suffix or path-segment match.
    let leaf = parts.last().copied().unwrap_or("");
    let path_segments = &parts[..];
    let cadence = if leaf.ends_with("_weekly") || path_segments.contains(&"weekly") {
        Some("weekly".to_string())
    } else if leaf.ends_with("_daily") || path_segments.contains(&"daily") {
        Some("daily".to_string())
    } else {
        None
    };

    (category, subcategory, cadence)
}

/// Pull `(fqn, payload_b64)` tuples for every object of `kind`. Used by
/// the populate_* passes that need to walk binary payloads.
fn fetch_fqn_payloads(conn: &Connection, kind: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare("SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind = ?1")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([kind], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Parse the SPN-triple format that appears in quest payloads:
///
/// ```text
/// spn.<faction.planet.path>;<target_fqn>;<numeric_id>
/// ```
///
/// Returns all three parts, or None if the string is not a well-formed
/// SPN triple. Caller decides whether to keep based on `target_fqn`'s
/// prefix (npc/plc/etc.).
fn parse_spn_triple(s: &str) -> Option<(String, String, u64)> {
    if !s.starts_with("spn.") {
        return None;
    }
    let mut parts = s.splitn(3, ';');
    let spn_fqn = parts.next()?;
    let target_fqn = parts.next()?;
    let numeric_str = parts.next()?;
    let runtime_id = numeric_str.parse::<u64>().ok()?;
    Some((spn_fqn.to_string(), target_fqn.to_string(), runtime_id))
}

/// Convenience: extract just the npc.* target from an SPN triple, or None
/// if the triple is malformed or its target is not an NPC.
fn npc_from_spn_triple(s: &str) -> Option<String> {
    let (_spn, target, _id) = parse_spn_triple(s)?;
    if target.starts_with("npc.") {
        Some(target)
    } else {
        None
    }
}

impl Database {
    /// Resolve `a:enc.*` references in quest payloads to `npc.*` FQNs by
    /// scanning each referenced encounter's payload, then write rows into
    /// `quest_npcs`. Runs after quest tables are populated.
    ///
    /// Two-hop resolution: quest payload contains `a:enc.<faction>.<planet>...`
    /// strings; encounter object payload contains `npc.*` strings. The `a:`
    /// prefix is a payload-side type marker and is stripped before the lookup.
    pub fn populate_quest_npcs(&self) -> Result<u64> {
        use crate::pbuk::extract_strings_from_payload;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use std::collections::HashMap;

        // Pull encounter, spawn, and quest rows under one lock.
        let (enc_rows, spawn_rows, quest_rows) = {
            let conn = self.conn.lock().unwrap();
            let enc_rows = fetch_fqn_payloads(&conn, "Encounter")?;
            let spawn_rows = fetch_fqn_payloads(&conn, "Spawn")?;
            let quest_rows = fetch_fqn_payloads(&conn, "Quest")?;
            (enc_rows, spawn_rows, quest_rows)
        };

        // Build spn_fqn -> Vec<npc_fqn> by scanning each spawn payload once.
        // Spawns are the layer between encounters and NPCs: encounter payloads
        // reference `spn.*`, spawn payloads reference `npc.*`. Without this
        // map, the enc -> npc resolution finds only the small subset of
        // encounters that name NPCs directly (~166 of 9652).
        let mut spn_to_npcs: HashMap<String, Vec<String>> = HashMap::new();
        for (spn_fqn, payload_b64) in spawn_rows {
            let Ok(payload) = BASE64.decode(&payload_b64) else {
                continue;
            };
            let mut npcs: Vec<String> = extract_strings_from_payload(&payload)
                .into_iter()
                .filter(|s| s.starts_with("npc."))
                .collect();
            npcs.sort();
            npcs.dedup();
            if !npcs.is_empty() {
                spn_to_npcs.insert(spn_fqn, npcs);
            }
        }

        // Build enc_fqn -> Vec<npc_fqn>. An encounter's NPCs come from two
        // sources, joined together:
        //   1. npc.* strings directly in the encounter payload
        //   2. spn.* strings in the encounter payload, resolved via spn_to_npcs
        let mut enc_to_npcs: HashMap<String, Vec<String>> = HashMap::new();
        for (enc_fqn, payload_b64) in enc_rows {
            let Ok(payload) = BASE64.decode(&payload_b64) else {
                continue;
            };
            let strings = extract_strings_from_payload(&payload);
            let mut npcs: Vec<String> = Vec::new();
            for s in &strings {
                if s.starts_with("npc.") {
                    npcs.push(s.clone());
                } else if s.starts_with("spn.") {
                    if let Some(spn_npcs) = spn_to_npcs.get(s) {
                        npcs.extend(spn_npcs.iter().cloned());
                    } else {
                        // Some encounters reference a base spawn name like
                        // `spn.X.multi.isen` that the engine resolves at
                        // runtime to a variant (`isen_no_weapon`,
                        // `isen_captured`). Fall back to prefix-match on
                        // `<base>_*` so the underlying character resolves.
                        let prefix = format!("{}_", s);
                        for (spn_fqn, spn_npcs) in &spn_to_npcs {
                            if spn_fqn.starts_with(&prefix) {
                                npcs.extend(spn_npcs.iter().cloned());
                            }
                        }
                    }
                }
            }
            npcs.sort();
            npcs.dedup();
            if !npcs.is_empty() {
                enc_to_npcs.insert(enc_fqn, npcs);
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut npc_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO quest_npcs (quest_fqn, npc_fqn) VALUES (?1, ?2)",
        )?;

        let mut link_count = 0u64;
        for (quest_fqn, payload_b64) in &quest_rows {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };
            let strings = extract_strings_from_payload(&payload);

            let mut seen_pairs = std::collections::HashSet::new();
            let mut emit = |npc_fqn: String, count: &mut u64| -> Result<()> {
                if seen_pairs.insert((quest_fqn.clone(), npc_fqn.clone())) {
                    npc_stmt.execute(rusqlite::params![quest_fqn, npc_fqn])?;
                    *count += 1;
                }
                Ok(())
            };

            for s in &strings {
                // Path 1: SPN triple in quest payload -- `spn.X;npc.Y;<numeric_id>`.
                // The middle segment is the NPC that spawns at this point. This
                // is the direct quest -> npc reference path.
                if let Some(npc_fqn) = npc_from_spn_triple(s) {
                    emit(npc_fqn, &mut link_count)?;
                    continue;
                }

                // Path 2: encounter reference (`a:enc.*` or `enc.*`) -- two-hop
                // resolution through enc_to_npcs map. Encounters often spawn
                // NPCs that the quest does not name directly.
                let enc_fqn = match s.strip_prefix("a:") {
                    Some(rest) if rest.starts_with("enc.") => rest,
                    _ if s.starts_with("enc.") => s.as_str(),
                    _ => continue,
                };
                if let Some(npcs) = enc_to_npcs.get(enc_fqn) {
                    for npc_fqn in npcs {
                        emit(npc_fqn.clone(), &mut link_count)?;
                    }
                }
            }
        }

        drop(npc_stmt);
        tx.commit()?;
        Ok(link_count)
    }

    /// Extract every SPN triple (`spn.X;target.Y;<numeric>`) from quest
    /// payloads and write rows into `spawn_runtime_ids`. The numeric is
    /// kept as-is for the combat-log bridge: it may be a runtime node ID,
    /// packed coordinates, or both. Decoding waits on combat log capture
    /// (#20).
    pub fn populate_spawn_runtime_ids(&self) -> Result<u64> {
        use crate::pbuk::extract_strings_from_payload;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let quest_rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            fetch_fqn_payloads(&conn, "Quest")?
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO spawn_runtime_ids (spn_fqn, target_fqn, runtime_id) VALUES (?1, ?2, ?3)",
        )?;

        let mut count = 0u64;
        for (_quest_fqn, payload_b64) in &quest_rows {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };
            for s in extract_strings_from_payload(&payload) {
                if let Some((spn_fqn, target_fqn, runtime_id)) = parse_spn_triple(&s) {
                    stmt.execute(rusqlite::params![spn_fqn, target_fqn, runtime_id as i64,])?;
                    count += 1;
                }
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Extract `quest_reward_*` variable names from each quest payload and
    /// write rows into `quest_rewards`. Variable names are categories
    /// (adrenal, medpac, alignment, gift); specific items are runtime-resolved
    /// by the engine and not in payload data.
    pub fn populate_quest_rewards(&self) -> Result<u64> {
        use crate::pbuk::extract_strings_from_payload;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let quest_rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            fetch_fqn_payloads(&conn, "Quest")?
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO quest_rewards (quest_fqn, reward_variable) VALUES (?1, ?2)",
        )?;

        let mut count = 0u64;
        for (quest_fqn, payload_b64) in &quest_rows {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };
            for s in extract_strings_from_payload(&payload) {
                if s.starts_with("quest_reward_") {
                    stmt.execute(rusqlite::params![quest_fqn, s])?;
                    count += 1;
                }
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `missions` from two sources:
    ///
    /// 1. Every `qst.*` object becomes a row with `source='qst'`.
    /// 2. Every unique mpn-prefix (path with the leaf phase segment dropped)
    ///    that does not already exist as a qst.* counterpart becomes a row
    ///    with `source='mpn-prefix'`.
    ///
    /// The mpn-prefix derivation: for `mpn.A.B.C.D`, the mission identity
    /// is `mpn.A.B.C` (drop the last segment). The qst.* counterpart check
    /// rewrites `mpn.X` -> `qst.X` and looks for that fqn in the qst set.
    pub fn populate_missions(&self) -> Result<u64> {
        use std::collections::HashSet;

        let (qst_fqns, phase_fqns): (Vec<String>, Vec<String>) = {
            let conn = self.conn.lock().unwrap();

            let mut qst_stmt = conn.prepare("SELECT fqn FROM objects WHERE kind = 'Quest'")?;
            let qst_fqns: Vec<String> = qst_stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(qst_stmt);

            let mut phase_stmt = conn.prepare("SELECT fqn FROM objects WHERE kind = 'Phase'")?;
            let phase_fqns: Vec<String> = phase_stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(phase_stmt);

            (qst_fqns, phase_fqns)
        };

        let qst_set: HashSet<&str> = qst_fqns.iter().map(|s| s.as_str()).collect();

        // Derive mpn-prefix groupings: for each phase, drop the last segment
        // and compute the qst.* counterpart. Skip if a qst.* counterpart exists.
        let mut mpn_prefixes: HashSet<String> = HashSet::new();
        for phase in &phase_fqns {
            let Some(last_dot) = phase.rfind('.') else {
                continue;
            };
            let prefix = &phase[..last_dot];
            let qst_equivalent = format!("qst{}", &prefix[3..]);
            if qst_set.contains(qst_equivalent.as_str()) {
                continue;
            }
            mpn_prefixes.insert(prefix.to_string());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO missions (mission_fqn, source) VALUES (?1, ?2)",
        )?;

        let mut count = 0u64;
        for fqn in &qst_fqns {
            stmt.execute(rusqlite::params![fqn, "qst"])?;
            count += 1;
        }
        for prefix in &mpn_prefixes {
            stmt.execute(rusqlite::params![prefix, "mpn-prefix"])?;
            count += 1;
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `conquest_objectives` from `ach.conquests.*` achievements.
    /// Parses FQN segments to derive category, subcategory, and cadence.
    pub fn populate_conquest_objectives(&self) -> Result<u64> {
        let rows: Vec<(String, Option<u32>)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, string_id FROM objects WHERE kind = 'Achievement' AND fqn LIKE 'ach.conquests.%'",
            )?;
            let result: Vec<(String, Option<u32>)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<u32>>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);
            result
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO conquest_objectives (fqn, category, subcategory, cadence, string_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        let mut count = 0u64;
        for (fqn, string_id) in &rows {
            let (category, subcategory, cadence) = parse_conquest_fqn(fqn);
            stmt.execute(rusqlite::params![
                fqn,
                category,
                subcategory,
                cadence,
                string_id
            ])?;
            count += 1;
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `mission_npcs` and `mission_rewards` by walking each mission's
    /// phase tree and aggregating extractions across every payload.
    ///
    /// For `source='qst'`, the phase set is just the quest object itself.
    /// For `source='mpn-prefix'`, the phase set is every `mpn.<prefix>.*`
    /// child object's payload.
    ///
    /// NPC resolution reuses the three-hop logic (quest -> enc -> spn -> npc
    /// + SPN-triple direct + prefix-match fallback).
    ///
    /// Reward extraction is the same `quest_reward_*` scan.
    pub fn populate_mission_data(&self) -> Result<(u64, u64)> {
        use crate::pbuk::extract_strings_from_payload;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use std::collections::HashMap;

        // Pull mission identities and all encounter/spawn rows under one lock.
        let (missions, enc_rows, spawn_rows) = {
            let conn = self.conn.lock().unwrap();

            let mut mission_stmt = conn.prepare("SELECT mission_fqn, source FROM missions")?;
            let missions: Vec<(String, String)> = mission_stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            drop(mission_stmt);

            let enc_rows = fetch_fqn_payloads(&conn, "Encounter")?;
            let spawn_rows = fetch_fqn_payloads(&conn, "Spawn")?;

            (missions, enc_rows, spawn_rows)
        };

        // Build spn -> Vec<npc> map (same as populate_quest_npcs).
        let mut spn_to_npcs: HashMap<String, Vec<String>> = HashMap::new();
        for (spn_fqn, payload_b64) in spawn_rows {
            let Ok(payload) = BASE64.decode(&payload_b64) else {
                continue;
            };
            let mut npcs: Vec<String> = extract_strings_from_payload(&payload)
                .into_iter()
                .filter(|s| s.starts_with("npc."))
                .collect();
            npcs.sort();
            npcs.dedup();
            if !npcs.is_empty() {
                spn_to_npcs.insert(spn_fqn, npcs);
            }
        }

        // Build enc -> Vec<npc> from encounter payloads (npc directly + via spawn).
        let mut enc_to_npcs: HashMap<String, Vec<String>> = HashMap::new();
        for (enc_fqn, payload_b64) in enc_rows {
            let Ok(payload) = BASE64.decode(&payload_b64) else {
                continue;
            };
            let strings = extract_strings_from_payload(&payload);
            let mut npcs: Vec<String> = Vec::new();
            for s in &strings {
                if s.starts_with("npc.") {
                    npcs.push(s.clone());
                } else if s.starts_with("spn.") {
                    if let Some(extra) = spn_to_npcs.get(s) {
                        npcs.extend(extra.iter().cloned());
                    } else {
                        let prefix = format!("{}_", s);
                        for (spn_fqn, spn_npcs) in &spn_to_npcs {
                            if spn_fqn.starts_with(&prefix) {
                                npcs.extend(spn_npcs.iter().cloned());
                            }
                        }
                    }
                }
            }
            npcs.sort();
            npcs.dedup();
            if !npcs.is_empty() {
                enc_to_npcs.insert(enc_fqn, npcs);
            }
        }

        // Build mission_fqn -> Vec<payload_b64> from the mission's own payloads
        // (qst object itself, and any owned cross-namespace references).
        let mission_payloads: HashMap<String, Vec<String>> = {
            let conn = self.conn.lock().unwrap();
            let mut map: HashMap<String, Vec<String>> = HashMap::new();

            // qst-source: the quest's payload (contains SPN triples + enc refs).
            let mut qst_stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind = 'Quest'",
            )?;
            for (fqn, b64) in qst_stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
            {
                map.entry(fqn).or_default().push(b64);
            }
            drop(qst_stmt);

            map
        };

        // Build mission_fqn -> Vec<npc_fqn> from path-namespace co-location.
        // For each mission, find all npc/spn/enc objects whose FQN sits inside
        // the mission's path stem (e.g. mpn.location.ord_mantell.class.trooper.
        // mannett_point owns npc.location.ord_mantell.class.trooper.mannett_point.*).
        // mpn phase payloads themselves are empty of NPC refs, so path-namespace
        // is the primary signal for mpn-only missions.
        let mission_namespace_npcs: HashMap<String, Vec<String>> = {
            let conn = self.conn.lock().unwrap();

            // Pull all objects with FQNs we care about.
            let mut stmt = conn.prepare(
                "SELECT fqn, kind, json_extract(json, '$.payload_b64') FROM objects \
                 WHERE kind IN ('Npc', 'Spawn', 'Encounter')",
            )?;
            let rows: Vec<(String, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);

            // Build mission_stem -> mission_fqn map for prefix lookup.
            // Stem is the mission_fqn with its leading segment (qst./mpn.) stripped.
            let mut stem_to_mission: HashMap<String, String> = HashMap::new();
            for (mission_fqn, _) in &missions {
                if let Some(idx) = mission_fqn.find('.') {
                    let stem = &mission_fqn[idx + 1..];
                    stem_to_mission.insert(stem.to_string(), mission_fqn.clone());
                }
            }

            // For each candidate object, derive its stem (drop leading prefix),
            // and find the longest matching mission stem (greedy match).
            // Then resolve to NPC FQNs via direct/spawn/encounter scan.
            let mut map: HashMap<String, Vec<String>> = HashMap::new();
            for (fqn, kind, payload_b64) in rows {
                let Some(idx) = fqn.find('.') else { continue };
                let obj_stem = &fqn[idx + 1..];

                // Find a mission stem that is a prefix of this object's stem.
                // Walk from the longest possible prefix down to handle nested
                // namespaces correctly.
                let mut owning_mission: Option<&String> = None;
                let mut owning_len = 0usize;
                for (mission_stem, mission_fqn) in &stem_to_mission {
                    if obj_stem.starts_with(mission_stem)
                        && obj_stem.len() > mission_stem.len()
                        && obj_stem.as_bytes()[mission_stem.len()] == b'.'
                        && mission_stem.len() > owning_len
                    {
                        owning_mission = Some(mission_fqn);
                        owning_len = mission_stem.len();
                    }
                }
                let Some(mission_fqn) = owning_mission else {
                    continue;
                };

                let entry = map.entry(mission_fqn.clone()).or_default();
                match kind.as_str() {
                    "Npc" => entry.push(fqn.clone()),
                    "Spawn" => {
                        if let Ok(payload) = BASE64.decode(&payload_b64) {
                            for s in extract_strings_from_payload(&payload) {
                                if s.starts_with("npc.") {
                                    entry.push(s);
                                }
                            }
                        }
                    }
                    "Encounter" => {
                        if let Some(npcs) = enc_to_npcs.get(&fqn) {
                            entry.extend(npcs.iter().cloned());
                        }
                    }
                    _ => {}
                }
            }

            // Dedup each mission's npc list.
            for npcs in map.values_mut() {
                npcs.sort();
                npcs.dedup();
            }

            map
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut npc_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO mission_npcs (mission_fqn, npc_fqn) VALUES (?1, ?2)",
        )?;
        let mut reward_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO mission_rewards (mission_fqn, reward_variable) VALUES (?1, ?2)",
        )?;

        let mut npc_count = 0u64;
        let mut reward_count = 0u64;

        for (mission_fqn, _source) in &missions {
            let mut seen_npcs: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut seen_rewards: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            // Source 1: namespace co-located NPCs (any npc/spn/enc object whose
            // FQN sits inside the mission's path stem). Primary signal for
            // mpn-only missions.
            if let Some(npcs) = mission_namespace_npcs.get(mission_fqn) {
                for n in npcs {
                    seen_npcs.insert(n.clone());
                }
            }

            // Source 2: mission's own payload (catches cross-namespace refs
            // like J&E referencing Tremel from `npc...multi.overseer_tremel`).
            // Empty for mpn-only missions; rich for qst-source.
            if let Some(payloads) = mission_payloads.get(mission_fqn) {
                for payload_b64 in payloads {
                    let Ok(payload) = BASE64.decode(payload_b64) else {
                        continue;
                    };
                    for s in extract_strings_from_payload(&payload) {
                        if s.starts_with("npc.") {
                            seen_npcs.insert(s);
                            continue;
                        }
                        if let Some(npc) = npc_from_spn_triple(&s) {
                            seen_npcs.insert(npc);
                            continue;
                        }
                        let enc_fqn = match s.strip_prefix("a:") {
                            Some(rest) if rest.starts_with("enc.") => Some(rest.to_string()),
                            _ if s.starts_with("enc.") => Some(s.clone()),
                            _ => None,
                        };
                        if let Some(enc) = enc_fqn {
                            if let Some(npcs) = enc_to_npcs.get(&enc) {
                                for n in npcs {
                                    seen_npcs.insert(n.clone());
                                }
                            }
                            continue;
                        }
                        if s.starts_with("quest_reward_") {
                            seen_rewards.insert(s);
                        }
                    }
                }
            }

            for npc_fqn in &seen_npcs {
                npc_stmt.execute(rusqlite::params![mission_fqn, npc_fqn])?;
                npc_count += 1;
            }
            for reward_variable in &seen_rewards {
                reward_stmt.execute(rusqlite::params![mission_fqn, reward_variable])?;
                reward_count += 1;
            }
        }

        drop(npc_stmt);
        drop(reward_stmt);
        tx.commit()?;
        Ok((npc_count, reward_count))
    }

    /// Populate `disciplines` and `discipline_abilities` from `abl.{class}.skill.{discipline}.*` FQNs.
    ///
    /// Discipline FQN structure:
    ///   abl.{class}.skill.{discipline}.{name}              -> base/core ability
    ///   abl.{class}.skill.{discipline}.mods.passive.{name} -> passive
    ///   abl.{class}.skill.{discipline}.mods.tier2.{name}   -> choice (lvl 23)
    ///   abl.{class}.skill.{discipline}.mods.tier3.{name}   -> choice (lvl 39+)
    ///   abl.{class}.skill.{discipline}.mods.special.{name} -> special
    ///   abl.{class}.skill.utility.{name}                   -> utility (shared)
    ///   abl.{class}.skill.mods.tier1.{name}                -> shared mod
    pub fn populate_disciplines(&self) -> Result<(u64, u64)> {
        self.flush()?;

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT fqn, game_id FROM objects WHERE fqn LIKE 'abl.%.skill.%'")?;
            let result: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

        let mut disc_set: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();
        let mut abl_rows: Vec<(String, String, String, Option<u8>, String)> = Vec::new();

        for (fqn, game_id) in &rows {
            // abl.{class}.skill.{rest}
            let parts: Vec<&str> = fqn.split('.').collect();
            if parts.len() < 5 {
                continue;
            }
            let class_code = parts[1];
            let discipline_name;
            let fqn_prefix;
            let slot_type: &str;
            let tier_level: Option<u8>;

            // abl.{class}.skill.utility.{name} -> utility, no discipline
            if parts[3] == "utility" {
                discipline_name = "utility";
                fqn_prefix = format!("abl.{}.skill.utility", class_code);
                slot_type = "utility";
                tier_level = None;
            } else if parts[3] == "mods" {
                // abl.{class}.skill.mods.tierN.{name} -> shared mod
                discipline_name = "shared";
                fqn_prefix = format!("abl.{}.skill.mods", class_code);
                slot_type = "shared_mod";
                tier_level = tier_from_segment(parts.get(4).copied());
            } else {
                // abl.{class}.skill.{discipline}.*
                discipline_name = parts[3];
                fqn_prefix = format!("abl.{}.skill.{}", class_code, discipline_name);

                if parts.len() >= 7 && parts[4] == "mods" {
                    match parts[5] {
                        "passive" => {
                            slot_type = "passive";
                            tier_level = None;
                        }
                        "special" => {
                            slot_type = "special";
                            tier_level = None;
                        }
                        s if s.starts_with("tier") => {
                            slot_type = "choice";
                            tier_level = tier_from_segment(Some(s));
                        }
                        _ => {
                            slot_type = "mod";
                            tier_level = None;
                        }
                    }
                } else {
                    slot_type = "core";
                    tier_level = None;
                }
            }

            disc_set.insert((
                class_code.to_string(),
                discipline_name.to_string(),
                fqn_prefix.clone(),
            ));
            abl_rows.push((
                fqn_prefix,
                game_id.clone(),
                fqn.clone(),
                tier_level,
                slot_type.to_string(),
            ));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut disc_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO disciplines (class_code, discipline_name, fqn_prefix) VALUES (?1, ?2, ?3)",
            )?;
            for (class_code, discipline_name, fqn_prefix) in &disc_set {
                stmt.execute(params![class_code, discipline_name, fqn_prefix])?;
                disc_count += 1;
            }
        }

        let mut abl_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO discipline_abilities (discipline_fqn_prefix, ability_game_id, ability_fqn, tier_level, slot_type) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (disc_prefix, game_id, fqn, tier, slot) in &abl_rows {
                stmt.execute(params![disc_prefix, game_id, fqn, tier, slot])?;
                abl_count += 1;
            }
        }

        tx.commit()?;
        Ok((disc_count, abl_count))
    }

    /// Populate `talent_abilities` by decoding GUID refs from `tal.*` payloads.
    ///
    /// Pattern (from MAPPINGS.md): CC 17E2840B D001 CF E000 [8-byte GUID BE]
    /// 37% of talents reference 1-3 abilities this way.
    pub fn populate_talent_abilities(&self) -> Result<u64> {
        self.flush()?;

        // Load all talent payloads + their game_ids
        let talents: Vec<(String, String, Vec<u8>)> = {
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT game_id, fqn, json FROM objects WHERE kind = 'Talent'")?;
            let raw: Vec<(String, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            raw.into_iter()
                .filter_map(|(game_id, fqn, json_str)| {
                    let v: serde_json::Value = serde_json::from_str(&json_str).ok()?;
                    let b64 = v.get("payload_b64")?.as_str()?;
                    let payload = BASE64.decode(b64).ok()?;
                    Some((game_id, fqn, payload))
                })
                .collect()
        };

        // Build guid → (game_id, fqn) lookup from all objects
        let guid_map: std::collections::HashMap<String, (String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT guid, game_id, fqn FROM objects")?;
            let rows: Vec<(String, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter()
                .map(|(guid, game_id, fqn)| (guid.to_uppercase(), (game_id, fqn)))
                .collect()
        };

        let mut links: Vec<(String, String, String, Option<String>)> = Vec::new();

        for (talent_game_id, talent_fqn, payload) in &talents {
            // Scan for CC 17E2840B (or reversed: 0B84E217) followed by D0 01 CF E0 00 ...
            // Field marker bytes (stored as found in MAPPINGS.md): CC 17 E2 84 0B
            // After field marker: D0 01 (int8 = 1), then CF E0 00 XX XX XX XX XX XX
            let guids = extract_ability_guids_from_talent(payload);
            for guid_hex in guids {
                let entry = guid_map.get(&guid_hex);
                links.push((
                    talent_game_id.clone(),
                    talent_fqn.clone(),
                    entry
                        .as_ref()
                        .map_or(guid_hex.clone(), |(gid, _)| gid.clone()),
                    entry.map(|(_, fqn)| fqn.clone()),
                ));
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO talent_abilities (talent_game_id, talent_fqn, ability_game_id, ability_fqn) VALUES (?1, ?2, ?3, ?4)",
        )?;

        let mut count = 0u64;
        for (talent_game_id, talent_fqn, ability_game_id, ability_fqn) in &links {
            stmt.execute(params![
                talent_game_id,
                talent_fqn,
                ability_game_id,
                ability_fqn
            ])?;
            count += 1;
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    pub fn stats(&self) -> Result<Stats> {
        // Ensure all pending data is flushed before counting
        self.flush()?;

        let conn = self.conn.lock().unwrap();
        let quests: u64 = conn.query_row("SELECT COUNT(*) FROM quests", [], |row| row.get(0))?;
        let abilities: u64 =
            conn.query_row("SELECT COUNT(*) FROM abilities", [], |row| row.get(0))?;
        let items: u64 = conn.query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))?;
        let npcs: u64 = conn.query_row("SELECT COUNT(*) FROM npcs", [], |row| row.get(0))?;
        let strings: u64 = conn.query_row("SELECT COUNT(*) FROM strings", [], |row| row.get(0))?;
        let chain_links: u64 =
            conn.query_row("SELECT COUNT(*) FROM quest_chain", [], |row| row.get(0))?;
        let npc_links: u64 =
            conn.query_row("SELECT COUNT(*) FROM quest_npcs", [], |row| row.get(0))?;
        let reward_links: u64 =
            conn.query_row("SELECT COUNT(*) FROM quest_rewards", [], |row| row.get(0))?;
        let runtime_ids: u64 =
            conn.query_row("SELECT COUNT(*) FROM spawn_runtime_ids", [], |row| {
                row.get(0)
            })?;
        let missions: u64 =
            conn.query_row("SELECT COUNT(*) FROM missions", [], |row| row.get(0))?;
        let conquest_objectives: u64 =
            conn.query_row("SELECT COUNT(*) FROM conquest_objectives", [], |row| {
                row.get(0)
            })?;
        let mission_npcs: u64 =
            conn.query_row("SELECT COUNT(*) FROM mission_npcs", [], |row| row.get(0))?;
        let mission_rewards: u64 =
            conn.query_row("SELECT COUNT(*) FROM mission_rewards", [], |row| row.get(0))?;
        let disciplines: u64 =
            conn.query_row("SELECT COUNT(*) FROM disciplines", [], |row| row.get(0))?;
        let discipline_abilities: u64 =
            conn.query_row("SELECT COUNT(*) FROM discipline_abilities", [], |row| {
                row.get(0)
            })?;
        let talent_abilities: u64 =
            conn.query_row("SELECT COUNT(*) FROM talent_abilities", [], |row| {
                row.get(0)
            })?;

        Ok(Stats {
            quests,
            abilities,
            items,
            npcs,
            strings,
            chain_links,
            npc_links,
            reward_links,
            runtime_ids,
            missions,
            conquest_objectives,
            mission_npcs,
            mission_rewards,
            disciplines,
            discipline_abilities,
            talent_abilities,
        })
    }

    /// Build mapping from icon_name → Vec<(game_id, kind)> for all objects with icons.
    /// Returns ALL objects per icon (shared icons get multiple game_ids).
    pub fn get_icon_mapping(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>> {
        self.flush()?; // Ensure all pending objects are written

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT icon_name, game_id, kind FROM objects WHERE icon_name IS NOT NULL")?;

        let mut mapping: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        for row in rows {
            let (icon_name, game_id, kind) = row?;
            // Lowercase for case-insensitive matching with file paths
            mapping
                .entry(icon_name.to_lowercase())
                .or_default()
                .push((game_id, kind));
        }

        Ok(mapping)
    }

    /// Build fallback icon mappings for objects with NULL icon_name.
    /// Derives icon names from known FQN patterns where the game data
    /// doesn't embed the icon reference in the binary payload.
    ///
    /// Returns the same format as get_icon_mapping: icon_name -> Vec<(game_id, kind)>
    pub fn get_fqn_fallback_icons(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>> {
        self.flush()?;

        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT fqn, game_id, kind FROM objects WHERE icon_name IS NULL")?;

        let mut mapping: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        for row in rows {
            let (fqn, game_id, kind) = row?;
            if let Some(icon_name) = derive_icon_from_fqn(&fqn) {
                mapping.entry(icon_name).or_default().push((game_id, kind));
            }
        }

        Ok(mapping)
    }
}

/// Count quest steps by looking for branch/step/task patterns in payload strings.
/// Pattern: `_bX_sY_tZ` where X=branch, Y=step, Z=task.
fn count_quest_steps(json_str: &str) -> i32 {
    use regex::Regex;
    use std::sync::OnceLock;

    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"_b\d+_s(\d+)").unwrap());

    let mut max_step = 0i32;
    for caps in re.captures_iter(json_str) {
        if let Ok(n) = caps[1].parse::<i32>() {
            if n > max_step {
                max_step = n;
            }
        }
    }
    max_step
}

/// Derive an icon filename from a FQN for objects that lack embedded icon references.
///
/// Known patterns:
/// - Legacy perk gift bonuses: itm.mtx.lgc.prk.affection_bonus.gift_{N} -> legacyofaltruism{N}
/// - Legacy perk gift speed: itm.mtx.lgc.prk.affection_bonus.gift_speed_{N} -> legacyofaltruism1
/// - Legacy perk conversation: itm.mtx.lgc.prk.affection_bonus.conversation_{N} -> legacyofpersuasion{N}
fn derive_icon_from_fqn(fqn: &str) -> Option<String> {
    // Legacy Cartel Market perks: itm.mtx.lgc.prk.affection_bonus.*
    if let Some(suffix) = fqn.strip_prefix("itm.mtx.lgc.prk.affection_bonus.") {
        if let Some(n) = suffix.strip_prefix("gift_speed_") {
            // Gift speed perks all use the tier-1 altruism icon
            let _rank: u8 = n.parse().ok()?;
            return Some("legacyofaltruism1".to_string());
        }
        if let Some(n) = suffix.strip_prefix("gift_") {
            // Gift effectiveness: gift_1 -> legacyofaltruism1, etc.
            let rank: u8 = n.parse().ok()?;
            return Some(format!("legacyofaltruism{}", rank));
        }
        if let Some(n) = suffix.strip_prefix("conversation_") {
            // Conversation influence: conversation_1 -> legacyofpersuasion1, etc.
            let rank: u8 = n.parse().ok()?;
            return Some(format!("legacyofpersuasion{}", rank));
        }
    }

    // Legacy talent perks: tal.legacy.perk.companion_gift_{N}
    // These are the talent counterparts of the item perks above
    if let Some(suffix) = fqn.strip_prefix("tal.legacy.perk.") {
        if let Some(n) = suffix.strip_prefix("companion_gift_") {
            let rank: u8 = n.parse().ok()?;
            return Some(format!("legacyofaltruism{}", rank));
        }
        if let Some(n) = suffix.strip_prefix("companion_gift_speed_") {
            let _rank: u8 = n.parse().ok()?;
            return Some("legacyofaltruism1".to_string());
        }
        if let Some(n) = suffix.strip_prefix("conversation_influence_") {
            let rank: u8 = n.parse().ok()?;
            return Some(format!("legacyofpersuasion{}", rank));
        }
    }

    None
}

/// Decode tier level from FQN segment like "tier2" → 23, "tier3" → 39, etc.
/// Maps SWTOR's tier numbering to actual level requirements.
fn tier_from_segment(seg: Option<&str>) -> Option<u8> {
    match seg? {
        "tier1" => Some(15),
        "tier2" => Some(23),
        "tier3" => Some(39),
        "tier4" => Some(43),
        "tier5" => Some(51),
        "tier6" => Some(64),
        "tier7" => Some(68),
        "tier8" => Some(73),
        _ => None,
    }
}

/// Extract ability GUIDs from a `tal.*` payload using the documented pattern:
///   CC 17 E2 84 0B  (field marker, may appear as CC 0B 84 E2 17 in some payloads)
///   D0 01           (int8 = 1)
///   CF E0 00 XX XX XX XX XX XX  (CF type tag + 8-byte GUID; E0 00 are GUID bytes 1-2)
///
/// Returns hex strings (uppercase, 16 chars) matching the objects.guid format.
fn extract_ability_guids_from_talent(payload: &[u8]) -> Vec<String> {
    let mut guids = Vec::new();
    let len = payload.len();
    if len < 16 {
        return guids;
    }

    let mut i = 0;
    while i + 16 <= len {
        // Look for CC followed by field ID 17E2840B (either byte order)
        if payload[i] != 0xCC {
            i += 1;
            continue;
        }
        let is_field = (i + 5 <= len)
            && ((payload[i + 1] == 0x17
                && payload[i + 2] == 0xE2
                && payload[i + 3] == 0x84
                && payload[i + 4] == 0x0B)
                || (payload[i + 1] == 0x0B
                    && payload[i + 2] == 0x84
                    && payload[i + 3] == 0xE2
                    && payload[i + 4] == 0x17));
        if !is_field {
            i += 1;
            continue;
        }
        // Skip CC + 4-byte field ID + D0 01
        let after = i + 5;
        if after + 2 > len {
            i += 1;
            continue;
        }
        // D0 01 marker
        let guid_start = if payload[after] == 0xD0 && payload[after + 1] == 0x01 {
            after + 2
        } else {
            after
        };
        // CF [8-byte GUID BE]: E0 00 are the first two bytes of the GUID, not markers.
        // Matches populate_quest_chain's format: payload[i+1..i+9] byte-concat hex.
        if guid_start + 9 > len {
            i += 1;
            continue;
        }
        if payload[guid_start] == 0xCF
            && payload[guid_start + 1] == 0xE0
            && payload[guid_start + 2] == 0x00
        {
            let g = &payload[guid_start + 1..guid_start + 9];
            let hex = g.iter().map(|b| format!("{b:02X}")).collect::<String>();
            guids.push(hex);
            i = guid_start + 9;
        } else {
            i += 1;
        }
    }
    guids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spn_triple_extracts_all_three_parts() {
        let s = "spn.location.korriban.foo;npc.location.korriban.bar;291310451818496";
        let (spn, target, id) = parse_spn_triple(s).unwrap();
        assert_eq!(spn, "spn.location.korriban.foo");
        assert_eq!(target, "npc.location.korriban.bar");
        assert_eq!(id, 291310451818496);
    }

    #[test]
    fn parse_spn_triple_rejects_missing_runtime_id() {
        // Two segments only, no numeric.
        assert!(parse_spn_triple("spn.X;npc.Y").is_none());
    }

    #[test]
    fn parse_spn_triple_rejects_non_numeric_third_segment() {
        assert!(parse_spn_triple("spn.X;npc.Y;not_a_number").is_none());
    }

    #[test]
    fn npc_from_spn_triple_extracts_middle_segment() {
        let s = "spn.location.korriban.class.sith_warrior.judge_and_executioner.jailer_knash;npc.location.korriban.class.sith_warrior.judge_and_executioner.jailer_knash;291310451818496";
        assert_eq!(
            npc_from_spn_triple(s).as_deref(),
            Some("npc.location.korriban.class.sith_warrior.judge_and_executioner.jailer_knash")
        );
    }

    #[test]
    fn npc_from_spn_triple_rejects_non_spn_strings() {
        assert!(npc_from_spn_triple("npc.korriban.foo").is_none());
        assert!(npc_from_spn_triple("a:enc.korriban.tomb").is_none());
        assert!(npc_from_spn_triple("Always").is_none());
    }

    #[test]
    fn npc_from_spn_triple_rejects_non_npc_targets() {
        // Spawn triples can also reference plc.* (placeables); this helper is
        // scoped to NPC-only and must reject them.
        let s = "spn.korriban.x;plc.korriban.carving;123";
        assert!(npc_from_spn_triple(s).is_none());
    }

    #[test]
    fn conquest_fqn_class_with_subcategory() {
        let (cat, sub, cad) =
            parse_conquest_fqn("ach.conquests.class.bounty_hunter.abilities.carbonize");
        assert_eq!(cat, "class");
        assert_eq!(sub.as_deref(), Some("bounty_hunter"));
        assert_eq!(cad, None);
    }

    #[test]
    fn conquest_fqn_location_with_planet() {
        let (cat, sub, _) =
            parse_conquest_fqn("ach.conquests.location.tatooine.complete_any_mission");
        assert_eq!(cat, "location");
        assert_eq!(sub.as_deref(), Some("tatooine"));
    }

    #[test]
    fn conquest_fqn_weekly_suffix() {
        let (_, _, cad) = parse_conquest_fqn("ach.conquests.crafting.craft_any_weekly");
        assert_eq!(cad.as_deref(), Some("weekly"));
    }

    #[test]
    fn conquest_fqn_weekly_segment_in_path() {
        let (cat, _, cad) = parse_conquest_fqn(
            "ach.conquests.galactic_seasons.priority_objectives.weekly.fp_vet_hutt",
        );
        assert_eq!(cat, "galactic_seasons");
        assert_eq!(cad.as_deref(), Some("weekly"));
    }

    #[test]
    fn conquest_fqn_daily_segment_in_path() {
        let (_, _, cad) = parse_conquest_fqn(
            "ach.conquests.galactic_seasons.priority_objectives.daily.heroics_out_rim",
        );
        assert_eq!(cad.as_deref(), Some("daily"));
    }

    #[test]
    fn conquest_fqn_rejects_non_conquest() {
        let (cat, _, _) = parse_conquest_fqn("ach.alliance.alliance_growth.specialists.x");
        assert_eq!(cat, "unknown");
    }
}
