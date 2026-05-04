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
    pub alignment_event: u64,
}

/// Serialized object ready for batch insert
struct PendingObject {
    game_id: String,
    stable_id: String,
    payload_hash: String,
    guid: String,
    template_guid: String,
    fqn: String,
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
    pub origins: u64,
    pub combat_styles: u64,
    pub combat_style_shared_abilities: u64,
    pub class_utility_talents: u64,
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

            -- Quest clusters: derived groupings to support bulk curation.
            -- Each quest gets one row per cluster_kind it matches; a quest can
            -- belong to multiple clusters (e.g. "class_act" and "class_planet"
            -- for the same FQN).
            --   cluster_kind:
            --     class_act        -- (faction, class, act_N)
            --     class_planet     -- qst.location.<planet>.class.<class>
            --     world_arc_hub    -- (exp.NN, planet, faction, hub_N)
            --     world_arc        -- (exp.NN, planet, world_arc, faction)
            --     planet_world     -- qst.location.<planet>.world.<faction>
            --     expansion_arc    -- (exp.NN, planet, arc_segment)
            --     event            -- qst.event.<event_name>
            --     alliance         -- qst.alliance.<arc>
            --     companion        -- qst.alliance.companion.<class>
            --     flashpoint       -- qst.flashpoint.<name>
            --     operation        -- qst.operation.<name>
            --     daily_area       -- qst.daily_area.<planet>
            --     heroic           -- qst.heroic.<name>
            --     qtr              -- qst.qtr.* (weekly conquests)
            --     ventures         -- qst.ventures.*
            --     galactic_seasons -- qst.exp.galactic_seasons.<season>
            CREATE TABLE IF NOT EXISTS quest_clusters (
                quest_fqn TEXT NOT NULL,
                cluster_kind TEXT NOT NULL,
                cluster_id TEXT NOT NULL,
                PRIMARY KEY (quest_fqn, cluster_kind, cluster_id)
            );
            CREATE INDEX IF NOT EXISTS idx_quest_clusters_id ON quest_clusters(cluster_id);
            CREATE INDEX IF NOT EXISTS idx_quest_clusters_kind ON quest_clusters(cluster_kind);

            -- Per-conversation counts of alignment-event tokens found in NODE
            -- bytes. SWTOR encodes alignment-coded dialog beats by attaching
            -- audio/effect event names like `event.darkmoment_NN`,
            -- `event.bigdarkmoment_NN`, `event.sinistermoment_NN`,
            -- `event.heroicmoment_NN`, `event.darksidetheme.*`,
            -- `event.lightsidetheme.*`, plus explicit `alignment_override` and
            -- `influence_desync` tokens. The presence and count of each kind
            -- is a coarse signal for the LS/DS/influence character of the
            -- dialog, even though the per-choice magnitudes (LS+50/+100, etc)
            -- are not yet decoded.
            --   event_kind:
            --     darkmoment        small DS choice trigger
            --     bigdarkmoment     major DS choice trigger
            --     sinistermoment    DS choice trigger
            --     darksidetheme     DS music theme setter
            --     heroicmoment      LS choice trigger
            --     lightsidetheme    LS music theme setter
            --     alignment_override explicit alignment override
            --     influence_desync  companion influence event
            --     affection_bot     companion affection-bot reaction
            CREATE TABLE IF NOT EXISTS conversation_alignment_events (
                cnv_fqn TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                event_count INTEGER NOT NULL,
                PRIMARY KEY (cnv_fqn, event_kind)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_align_kind ON conversation_alignment_events(event_kind);

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

            -- Quest chain links (built from GUID refs and prereq graph).
            -- Both endpoints are objects.game_id values (sha256(fqn:guid)[0:16]).
            CREATE TABLE IF NOT EXISTS quest_chain (
                source_game_id TEXT NOT NULL,
                target_game_id TEXT NOT NULL,
                link_type TEXT NOT NULL,
                PRIMARY KEY (source_game_id, target_game_id),
                FOREIGN KEY (source_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (target_game_id) REFERENCES objects(game_id)
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

            -- Disciplines: one row per combat discipline. After the PR3
            -- rework (#94 follow-up), per-origin shared and utility pools
            -- no longer live here -- they go to combat_style_shared_abilities
            -- and class_utility_talents respectively.
            --
            -- Two keys: origin_codename (mechanical from FQN -- abl.<origin>.skill.*)
            -- and combat_style_codename (from a hardcoded 48-row map: stable
            -- since 4.0, e.g. vengeance->juggernaut, watchman->sentinel).
            -- huttspawn ETL uses origin for faction routing + nav grouping
            -- and combat_style for editorial joins to combat_style_shared_abilities
            -- / class_utility_talents.
            CREATE TABLE IF NOT EXISTS disciplines (
                origin_codename        TEXT NOT NULL,
                discipline_name        TEXT NOT NULL,
                fqn_prefix             TEXT NOT NULL UNIQUE,  -- e.g. "abl.jedi_knight.skill.defense"
                combat_style_codename  TEXT NOT NULL,
                PRIMARY KEY (origin_codename, discipline_name),
                FOREIGN KEY (combat_style_codename) REFERENCES combat_styles(fqn_segment)
            );

            CREATE INDEX IF NOT EXISTS idx_disciplines_origin ON disciplines(origin_codename);
            CREATE INDEX IF NOT EXISTS idx_disciplines_combat_style ON disciplines(combat_style_codename);

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
                PRIMARY KEY (discipline_fqn_prefix, ability_game_id),
                FOREIGN KEY (ability_game_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_discipline_abilities_disc ON discipline_abilities(discipline_fqn_prefix);
            CREATE INDEX IF NOT EXISTS idx_discipline_abilities_abl  ON discipline_abilities(ability_game_id);

            -- Discipline talents: every tal.* that belongs to a discipline.
            -- Mapping is mechanical from FQN: a talent at
            --   tal.<class>.skill.<segment>.<name>
            -- maps to the discipline whose fqn_prefix is
            --   abl.<class>.skill.<segment>
            -- This includes the per-class shared utility discipline
            -- (tal.<class>.skill.utility.*) -- consumers fold those into
            -- combat-discipline trees editorially if they want.
            --
            -- No tier_level column. SWTOR's discipline-tree tier coordinates
            -- (which talent sits at which level/column on screen) are not
            -- encoded in tal.* payloads or FQN segments -- that's editorial
            -- tree layout that lives outside kessel's source data. The
            -- junction is mechanical membership only; trees rendered from
            -- it need a separate curated layout layer.
            CREATE TABLE IF NOT EXISTS discipline_talents (
                discipline_fqn_prefix  TEXT NOT NULL,
                talent_game_id         TEXT NOT NULL,
                talent_fqn             TEXT NOT NULL,
                PRIMARY KEY (discipline_fqn_prefix, talent_game_id),
                FOREIGN KEY (talent_game_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_discipline_talents_disc ON discipline_talents(discipline_fqn_prefix);
            CREATE INDEX IF NOT EXISTS idx_discipline_talents_tal  ON discipline_talents(talent_game_id);

            -- Combat-style-level shared ability pool (#94 PR3 rework).
            -- Replaces the per-discipline fan-out previously emitted into
            -- discipline_abilities for class-shared (`abl.<origin>.<name>`),
            -- skill-utility (`abl.<origin>.skill.utility.*`), and shared-mod
            -- (`abl.<origin>.skill.mods.tierN.*`) abilities. Each origin's
            -- shared abilities fan to BOTH combat styles of that origin --
            -- e.g. Force Leap (sith_warrior class-shared) emits one row for
            -- juggernaut and one for marauder.
            --
            -- slot_type vocabulary: 'class_shared' | 'utility' | 'shared_mod'.
            -- Open TEXT (no CHECK) per D3 sign-off; expected subset documented
            -- here.
            CREATE TABLE IF NOT EXISTS combat_style_shared_abilities (
                combat_style_codename  TEXT NOT NULL,
                ability_game_id        TEXT NOT NULL,
                ability_fqn            TEXT NOT NULL,
                slot_type              TEXT NOT NULL,
                PRIMARY KEY (combat_style_codename, ability_game_id, slot_type),
                FOREIGN KEY (ability_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (combat_style_codename) REFERENCES combat_styles(fqn_segment)
            );

            CREATE INDEX IF NOT EXISTS idx_combat_style_shared_abilities_style
                ON combat_style_shared_abilities(combat_style_codename);
            CREATE INDEX IF NOT EXISTS idx_combat_style_shared_abilities_abl
                ON combat_style_shared_abilities(ability_game_id);

            -- Combat-style-level utility talent pool (#94 PR3 rework).
            -- Replaces the per-discipline fan-out previously emitted into
            -- discipline_talents for `tal.<origin>.skill.utility.*`. Each
            -- origin's utility talents fan to BOTH combat styles of that
            -- origin -- huttspawn ETL just reads, no FQN re-derivation.
            CREATE TABLE IF NOT EXISTS class_utility_talents (
                combat_style_codename  TEXT NOT NULL,
                talent_game_id         TEXT NOT NULL,
                talent_fqn             TEXT NOT NULL,
                PRIMARY KEY (combat_style_codename, talent_game_id),
                FOREIGN KEY (talent_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (combat_style_codename) REFERENCES combat_styles(fqn_segment)
            );

            CREATE INDEX IF NOT EXISTS idx_class_utility_talents_style
                ON class_utility_talents(combat_style_codename);
            CREATE INDEX IF NOT EXISTS idx_class_utility_talents_talent
                ON class_utility_talents(talent_game_id);

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

            -- Talent classification + script-hook decode from tal.* GOM
            -- payload (#70). resource_pool mirrors ability_stats (rage,
            -- focus, force, heat, ammo, energy, gsf). tier is the FQN's
            -- last segment (tier1, tier_3a, base, passive, etc) — useful
            -- for grouping discipline-tree tiers and GSF upgrade tiers
            -- without a second lookup. script_hook is the length-prefixed
            -- ASCII tail string at the end of the talent payload (vault
            -- MAPPINGS.md lines 339-365); it identifies the underlying
            -- ability mod the talent triggers (e.g. abl_bh_me_kolto_shot,
            -- spvp_reducedcooldown, iamilitaryofficer). 94% of talents
            -- have one.
            CREATE TABLE IF NOT EXISTS talent_details (
                talent_game_id  TEXT PRIMARY KEY,
                resource_pool   TEXT,    -- force | rage | focus | heat | ammo | energy | gsf | NULL
                tier            TEXT,    -- FQN last segment
                script_hook     TEXT,    -- payload tail string, NULL if none
                FOREIGN KEY (talent_game_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_talent_details_pool ON talent_details(resource_pool);
            CREATE INDEX IF NOT EXISTS idx_talent_details_hook ON talent_details(script_hook);

            -- Ability stats decoded from abl.* GOM payload (#69, refined #74).
            -- Properties live as a contiguous run of [u16 LE propId][f32 LE
            -- value] records starting at the sentinel 01 04 00 00 80 BF
            -- (= 0x0401 = -1.0, an uninit marker). The block ends where the
            -- next 2 bytes are not in 0x04xx. Walker is sentinel-anchored,
            -- which eliminates the brute-force false positives the v1 scan
            -- produced (force_cost=1 on warrior/agent abilities, etc).
            -- Resource costs use a value threshold (>=5) since 0x0403 also
            -- appears at low values for non-Force tech abilities as a
            -- scaling coefficient, not a cost.
            CREATE TABLE IF NOT EXISTS ability_stats (
                ability_game_id     TEXT PRIMARY KEY,
                resource_pool       TEXT,    -- force | rage | focus | heat | ammo | energy | NULL
                cooldown            REAL,    -- 0x0401, seconds
                cast_time           REAL,    -- 0x041b, seconds (cast or channel)
                channel_duration    REAL,    -- 0x0406, seconds
                hard_cast_time      REAL,    -- 0x041a, alternate cast prop
                force_cost          INTEGER, -- 0x0403, Force-pool cost (sorcerer/sage)
                resource_cost       INTEGER, -- 0x041e, heat/energy/ammo cost (tech)
                raw_props           TEXT,    -- JSON {hex_id: f32} for all in-block 0x04xx records
                FOREIGN KEY (ability_game_id) REFERENCES objects(game_id)
            );

            -- GSF talent stats decoded from tal.spvp.* GOM payloads (#80).
            -- Records have shape `[c9 01]? <stat_id:u8> 01 04 <f32 LE>` and end
            -- at the signature `cb 19 d7 4b ?? 03`. Stat IDs are decoded via
            -- the embedded gsf_stat_dictionary.toml; rows ship with plain
            -- labels and units so consumers query
            --   WHERE label = 'cooldown_delta_seconds'
            -- rather than a hex byte. `confidence` is verified | guess |
            -- unknown -- callers can filter to verified-only data.
            -- `rank` preserves rank-progression ordering when a single talent
            -- payload encodes multiple records of the same stat (e.g.
            -- engine_power_regen.upgrade emits +4/+8/+12 as three rows).
            CREATE TABLE IF NOT EXISTS gsf_talent_stats (
                talent_game_id  TEXT NOT NULL,
                label           TEXT NOT NULL,
                unit            TEXT NOT NULL,
                rank            INTEGER NOT NULL,
                value           REAL NOT NULL,
                confidence      TEXT NOT NULL,
                stat_id         INTEGER NOT NULL,  -- raw byte for forensics
                PRIMARY KEY (talent_game_id, label, rank),
                FOREIGN KEY (talent_game_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_gsf_talent_stats_label
                ON gsf_talent_stats(label);

            -- GSF base ability stats decoded from abl.spvp.* payloads (#78).
            -- Same shape as gsf_talent_stats: labels and units come from the
            -- embedded dictionary. prop_id semantics differ from ground
            -- abilities (0x0402 = cooldown for GSF, animation marker for
            -- ground), so the dictionary has a separate ability_stats section.
            CREATE TABLE IF NOT EXISTS gsf_ability_stats (
                ability_game_id TEXT NOT NULL,
                label           TEXT NOT NULL,
                unit            TEXT NOT NULL,
                rank            INTEGER NOT NULL,
                value           REAL NOT NULL,
                confidence      TEXT NOT NULL,
                prop_id         INTEGER NOT NULL,  -- raw u16 for forensics
                PRIMARY KEY (ability_game_id, label, rank),
                FOREIGN KEY (ability_game_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_gsf_ability_stats_label
                ON gsf_ability_stats(label);

            -- Class taxonomy (#94).
            -- Post-7.0 the system is flat: 8 origins (the legacy classes,
            -- "story" choice in character creation), 16 combat styles (the
            -- legacy advanced classes, "playstyle" choice). Eligibility =
            -- matching `attack_type` (force vs tech). No join table needed.
            --
            -- Origins have no top-level GOM object; they live as the second
            -- FQN segment in many other prefixes (`qst.class.<origin>.*`,
            -- `apc.legacy.class.<origin>.*`, etc.). Rows are derived from
            -- the canonical PLAYER_CLASSES list. `string_id` resolves via
            -- `cdx.game_rules.classes.<fqn_segment>` -- a clean 8-row codex
            -- block whose trailing FQN segment matches our origin codenames
            -- exactly (agent, sith_inquisitor, etc.).
            --
            -- Combat styles ARE GOM objects at `class.pc.advanced.<style>`.
            -- Two have internal codenames -- force_wizard = sage, specialist
            -- = vanguard. Display strings come from `cdx.advanced_classes.<style>`,
            -- which is on a different object; we resolve the cdx string_id
            -- and store it on the combat_styles row directly.
            CREATE TABLE IF NOT EXISTS origins (
                fqn_segment    TEXT PRIMARY KEY,    -- 'sith_warrior', 'agent', etc.
                faction        TEXT NOT NULL,      -- 'empire' | 'republic'
                attack_type    TEXT NOT NULL,      -- 'force' | 'tech'
                string_id      INTEGER             -- -> strings.id2 (NULL until source confirmed)
            );

            CREATE INDEX IF NOT EXISTS idx_origins_attack ON origins(attack_type);

            CREATE TABLE IF NOT EXISTS combat_styles (
                combat_style_id TEXT PRIMARY KEY,   -- game_id of class.pc.advanced.<style>
                fqn             TEXT NOT NULL UNIQUE,
                fqn_segment     TEXT NOT NULL UNIQUE,     -- internal name ('force_wizard', 'specialist', ...)
                display_segment TEXT NOT NULL,     -- canonical name ('sage', 'vanguard', ...)
                faction         TEXT NOT NULL,     -- 'empire' | 'republic' (legacy adv-class faction)
                attack_type     TEXT NOT NULL,     -- 'force' | 'tech'
                string_id       INTEGER,           -- -> strings.id2 from cdx.advanced_classes.<display>
                FOREIGN KEY (combat_style_id) REFERENCES objects(game_id)
            );

            CREATE INDEX IF NOT EXISTS idx_combat_styles_attack ON combat_styles(attack_type);
            CREATE INDEX IF NOT EXISTS idx_combat_styles_faction ON combat_styles(faction);
            CREATE INDEX IF NOT EXISTS idx_combat_styles_display ON combat_styles(display_segment);

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
            game_id: obj.game_id.clone(),
            stable_id: obj.stable_id.clone(),
            payload_hash: obj.payload_hash.clone(),
            guid: obj.guid.clone(),
            template_guid: obj.template_guid.clone(),
            fqn: obj.fqn.clone(),
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
                INSERT INTO objects (game_id, stable_id, payload_hash, guid, template_guid, fqn, kind, icon_name, string_id, for_export, version, revision, json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(game_id) DO UPDATE SET
                    stable_id = excluded.stable_id,
                    payload_hash = excluded.payload_hash,
                    guid = excluded.guid,
                    template_guid = excluded.template_guid,
                    fqn = excluded.fqn,
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
                    obj.game_id,
                    obj.stable_id,
                    obj.payload_hash,
                    obj.guid,
                    obj.template_guid,
                    obj.fqn,
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
    /// Mark one "best" row per FQN as canonical; demote the rest.
    ///
    /// During extraction, the same FQN can appear under multiple GUIDs --
    /// canonical objects with full payload, plus stub references that share
    /// the FQN. The in-memory accept_variant filter blocks inferior variants
    /// that follow a superior one, but cannot retroactively demote a stub
    /// that was inserted before the canonical version arrived.
    ///
    /// This pass picks the highest "extraction quality" row per FQN (non-NULL
    /// string_id, then non-NULL icon_name, then larger json payload, then
    /// guid ASC for stable ordering), sets `is_canonical = 1` on it and
    /// `is_canonical = 0` on the others. Nothing is deleted -- inferior
    /// variants remain available to delta tooling and forensics.
    ///
    /// Consumers that previously relied on dedup-by-DELETE should filter
    /// `WHERE is_canonical = 1` to get the same set of rows.
    ///
    /// Returns the count of rows demoted (`is_canonical` flipped from 1 to 0).
    pub fn mark_canonical_by_fqn(&self) -> Result<u64> {
        self.flush()?;
        let conn = self.conn.lock().unwrap();

        // Reset everyone to canonical, then demote losers. Idempotent: running
        // twice produces the same result.
        conn.execute("UPDATE objects SET is_canonical = 1", [])?;

        let demoted = conn.execute(
            r#"
            UPDATE objects SET is_canonical = 0
            WHERE game_id IN (
                SELECT game_id FROM (
                    SELECT game_id, ROW_NUMBER() OVER (
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

        Ok(demoted as u64)
    }

    /// Backfill `objects.string_id` for canonical `abl.*` rows whose payload
    /// did not carry a CE/string-table marker (so neither
    /// `extract_string_id_via_fqn_with` nor `extract_string_id_via_type_marker`
    /// could recover the linkage at extraction time).
    ///
    /// Strategy: the ability's display name is recoverable from the FQN's
    /// last segment (snake_case -> Title Case). For each candidate row, look
    /// up STB strings whose text matches that name at id1=0 AND that have a
    /// description at id1=1 (real abilities have both; UI labels usually
    /// don't). Only commit when exactly one match remains and the candidate
    /// id2 is not already linked to another canonical object.
    ///
    /// Recovers a small set of discipline-pick abilities like
    /// `abl.smuggler.skill.saboteur.sabotage` whose GOM payload encodes
    /// effect references but omits the string-table type marker. Without
    /// this pass these rows ship with NULL string_id and cannot be joined
    /// to display-name strings via the standard `id1 = 0` pattern.
    ///
    /// Returns count of rows updated.
    pub fn backfill_missing_string_ids(&self) -> Result<u64> {
        self.flush()?;
        let conn = self.conn.lock().unwrap();

        // Gather candidate rows: canonical abl.* abilities with NULL
        // string_id. Only abl.* — talents and other kinds use different
        // string-linking patterns and should not be heuristically matched.
        let candidates: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn FROM objects \
                 WHERE kind = 'Ability' AND fqn LIKE 'abl.%' \
                   AND is_canonical = 1 AND string_id IS NULL",
            )?;
            let result: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

        if candidates.is_empty() {
            return Ok(0);
        }

        // Already-linked string_ids -- never reuse one across abilities;
        // a duplicate link would be a false positive on a homonym.
        let used: std::collections::HashSet<u32> = {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT string_id FROM objects \
                 WHERE string_id IS NOT NULL AND is_canonical = 1",
            )?;
            let result: std::collections::HashSet<u32> = stmt
                .query_map([], |row| row.get::<_, u32>(0))?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

        // Lookup by display name -- prepared once, executed per candidate.
        // Restrict to id2's that have BOTH a name (id1=0) AND a description
        // (id1=1). Real ability strings always have both; UI-label-only
        // entries don't, which filters out homonym noise.
        let mut find_stmt = conn.prepare(
            "SELECT s.id2 FROM strings s \
             WHERE s.text = ?1 AND s.locale = 'en-us' AND s.id1 = 0 \
               AND EXISTS ( \
                 SELECT 1 FROM strings d \
                 WHERE d.id2 = s.id2 AND d.id1 = 1 AND d.locale = 'en-us' \
               )",
        )?;

        let mut update_stmt =
            conn.prepare("UPDATE objects SET string_id = ?1 WHERE game_id = ?2")?;

        let mut updated = 0u64;
        for (game_id, fqn) in &candidates {
            let last_segment = match fqn.rsplit('.').next() {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            let expected_name = title_case_from_snake(last_segment);

            let matches: Vec<u32> = find_stmt
                .query_map([&expected_name], |row| row.get::<_, u32>(0))?
                .filter_map(|r| r.ok())
                .filter(|id| !used.contains(id))
                .collect();

            if matches.len() == 1 {
                update_stmt.execute(params![matches[0], game_id])?;
                updated += 1;
            }
        }

        Ok(updated)
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
                conn.prepare("SELECT fqn, string_id, json FROM objects WHERE kind = 'Quest' AND is_canonical = 1")?;
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

        // Fetch fqn + payload b64 for all Items. Payload is consulted only as
        // a fallback when classify() returns None for item_level (FQN had no
        // ilvl_NNNN segment). See item::extract_item_level_from_payload.
        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'Item' AND is_canonical = 1",
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0u64;

        {
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO item_details (fqn, item_kind, slot, weapon_type, armor_weight, rarity, item_level, source, is_schematic, crew_skill) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for (fqn, payload_b64) in &rows {
                let mut d = item::classify(fqn);
                if d.item_level.is_none() {
                    if let Ok(payload) = BASE64.decode(payload_b64) {
                        d.item_level = item::extract_item_level_from_payload(&payload);
                    }
                }
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
            let mut stmt = conn.prepare(
                "SELECT guid, game_id FROM objects WHERE fqn LIKE 'qst.%' AND is_canonical = 1",
            )?;
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
                 FROM objects WHERE fqn LIKE 'qst.%' AND is_canonical = 1",
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

    /// Populate `quest_chain` with `npc_giver` links derived from the
    /// conversation graph. For each pair of quests that share an NPC actor
    /// in their granting conversations, emit an edge -- the same NPC giving
    /// two quests means they're related in player flow.
    ///
    /// Filters to within-cluster pairs only (using quest_clusters): an NPC
    /// who appears in both a class story conversation and a flashpoint
    /// conversation isn't necessarily linking those two quests, but an NPC
    /// who appears in two conversations within the same `class_planet`
    /// bucket likely is.
    pub fn populate_quest_chain_npc_giver(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        // Use a single SQL pass with self-join to keep it simple and fast.
        // The within-cluster filter restricts the cartesian explosion.
        let n = tx.execute(
            "INSERT OR IGNORE INTO quest_chain (source_game_id, target_game_id, link_type) \
             SELECT DISTINCT \
                 oa.game_id AS source_game_id, \
                 ob.game_id AS target_game_id, \
                 'npc_giver' AS link_type \
             FROM conversation_quest_refs ra \
             JOIN conversation_npcs cna ON cna.cnv_fqn = ra.cnv_fqn \
             JOIN conversation_quest_refs rb \
                 ON rb.cnv_fqn IN ( \
                     SELECT cnv_fqn FROM conversation_npcs \
                     WHERE npc_fqn = cna.npc_fqn \
                 ) \
                 AND rb.quest_fqn != ra.quest_fqn \
             JOIN quest_clusters qca ON qca.quest_fqn = ra.quest_fqn \
             JOIN quest_clusters qcb \
                 ON qcb.quest_fqn = rb.quest_fqn \
                 AND qcb.cluster_kind = qca.cluster_kind \
                 AND qcb.cluster_id = qca.cluster_id \
             JOIN objects oa ON oa.fqn = ra.quest_fqn AND oa.kind='Quest' AND oa.is_canonical=1 \
             JOIN objects ob ON ob.fqn = rb.quest_fqn AND ob.kind='Quest' AND ob.is_canonical=1 \
             WHERE ra.quest_fqn < rb.quest_fqn",
            [],
        )? as u64;
        tx.commit()?;
        Ok(n)
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
                 FROM objects WHERE kind = 'schem' AND is_canonical = 1",
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
                "SELECT fqn FROM objects WHERE fqn LIKE 'itm.schem.%' AND kind = 'Item' AND is_canonical = 1",
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

    /// Populate `ability_stats` from `abl.*` GOM payloads (#74, refines #69).
    ///
    /// The dominant ability template (~86% of abl.* objects) writes properties
    /// as a contiguous run of 6-byte `[u16 LE prop_id][f32 LE value]` records
    /// starting at the sentinel `01 04 00 00 80 BF` (0x0401 = -1.0, an uninit
    /// marker). The block ends where the next 2 bytes are not in the 0x04xx
    /// range. Walking only that block eliminates the false positives the
    /// brute-force v1 scan produced (e.g. spurious force_cost=1 on warrior
    /// abilities from bytes outside the prop block).
    ///
    /// `resource_pool` is derived from the FQN class segment, not from the
    /// payload. Cost columns apply a value threshold (Force cost >= 5,
    /// resource cost >= 1) since 0x0403 is reused at low values as a scaling
    /// coefficient on tech abilities.
    ///
    /// Abilities on the secondary template (4000000002754EE0, ~459 rows
    /// including shock, endure_pain, takedown, companion abilities, racials,
    /// abl.space_combat.* on-rails missions, and abl.spvp.* Galactic
    /// Starfighter) have no sentinel-anchored block; they get a row only if
    /// their FQN resolves a `resource_pool` and no stats fields populate.
    pub fn populate_ability_stats(&self) -> Result<u64> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let conn = self.conn.lock().unwrap();

        let payloads: Vec<(String, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE fqn LIKE 'abl.%' AND is_canonical = 1",
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
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO ability_stats \
             (ability_game_id, resource_pool, cooldown, cast_time, \
              channel_duration, hard_cast_time, force_cost, resource_cost, \
              raw_props) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;

        let mut count: u64 = 0;
        for (game_id, fqn, payload_b64) in &payloads {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };
            let stats = scan_ability_props(&payload);
            let pool = resource_pool_from_fqn(fqn);
            if stats.any_hit() || pool.is_some() {
                stmt.execute(params![
                    game_id,
                    pool,
                    stats.cooldown,
                    stats.cast_time,
                    stats.channel_duration,
                    stats.hard_cast_time,
                    stats.force_cost,
                    stats.resource_cost,
                    stats.raw_props_json,
                ])?;
                count += 1;
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `talent_details` from `tal.*` payloads (#70).
    ///
    /// Three pieces of metadata per talent:
    ///   - `resource_pool` derived from FQN class (rage/focus/force/heat/
    ///     ammo/energy/gsf) — same vocabulary as `ability_stats`
    ///   - `tier` is the FQN's last segment (tier1, tier_3a, base, passive)
    ///   - `script_hook` is the length-prefixed ASCII tail string at the
    ///     end of the payload, identifying the ability mod the talent
    ///     triggers. ~94% of talents have one per vault MAPPINGS.md.
    ///
    /// A row is written for every `tal.*` object — the columns are NULL
    /// when not derivable.
    pub fn populate_talent_details(&self) -> Result<u64> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let conn = self.conn.lock().unwrap();

        let payloads: Vec<(String, String, Option<String>)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE fqn LIKE 'tal.%' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, String, Option<String>)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO talent_details \
             (talent_game_id, resource_pool, tier, script_hook) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        let mut count: u64 = 0;
        for (game_id, fqn, payload_b64) in &payloads {
            let pool = resource_pool_from_fqn(fqn);
            let tier = fqn.rsplit('.').next();
            let hook = payload_b64
                .as_deref()
                .and_then(|b64| BASE64.decode(b64).ok())
                .and_then(|payload| extract_talent_script_hook(&payload));
            stmt.execute(params![game_id, pool, tier, hook])?;
            count += 1;
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `gsf_talent_stats` from `tal.spvp.*` payloads (#80).
    ///
    /// GSF talent stat values are encoded distinctly from ground-ability
    /// `0x04xx` records -- they live as `[c9 01]? <stat_id> 01 04 <f32 LE>`
    /// records terminated by the signature `cb 19 d7 4b ?? 03`. The existing
    /// ability-stat extractor anchors on a sentinel that GSF talents do not
    /// carry, which is why GSF stat values were absent from spice.sqlite
    /// despite the talents themselves being present.
    ///
    /// Empirically, 250/350 GSF talents (71%) carry at least one record;
    /// 100 talents are flag-only (effects implemented on the parent ability
    /// or via script hook).
    pub fn populate_gsf_talent_stats(&self) -> Result<u64> {
        use crate::gsf_stat_dictionary::StatDictionary;
        use crate::schema::gsf_talent::decode_gsf_stats;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let dict = StatDictionary::from_embedded()?;
        let conn = self.conn.lock().unwrap();

        let payloads: Vec<(String, Option<String>)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE fqn LIKE 'tal.spvp.%' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, Option<String>)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO gsf_talent_stats \
             (talent_game_id, label, unit, rank, value, confidence, stat_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        let mut count: u64 = 0;
        for (game_id, payload_b64) in &payloads {
            let Some(b64) = payload_b64 else { continue };
            let Ok(payload) = BASE64.decode(b64) else {
                continue;
            };
            // Per-talent rank counter so multiple records of the same stat
            // (e.g. +4/+8/+12 rank progression) get rank=1,2,3 in payload
            // order. Different stats start at rank 1 independently.
            let mut rank_per_label: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            for rec in decode_gsf_stats(&payload) {
                let label = dict.talent_label(rec.stat_id);
                let rank = rank_per_label.entry(label.label.clone()).or_insert(0);
                *rank += 1;
                stmt.execute(params![
                    game_id,
                    label.label,
                    label.unit,
                    *rank,
                    rec.value as f64,
                    label.confidence,
                    rec.stat_id as i64,
                ])?;
                count += 1;
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `gsf_ability_stats` from `abl.spvp.*` payloads (#78).
    ///
    /// GSF base abilities reuse the ground `[u16 LE prop_id][f32 LE value]`
    /// layout but skip the `01 04 00 00 80 BF` cooldown sentinel that anchors
    /// `scan_ability_props`, and they scatter records across the payload
    /// rather than packing them contiguously. The decoder walks every 6-byte
    /// window and emits any record whose value is finite, non-zero, and in a
    /// reasonable magnitude range (subnormal-ish and huge values are
    /// byte-alignment noise from GUID/hash bytes).
    ///
    /// Empirical coverage: 112/131 abl.spvp.* abilities (85%) emit at least
    /// one record. Uncovered abilities are passive auras whose effects live
    /// on a parent activator or in script hooks.
    pub fn populate_gsf_ability_stats(&self) -> Result<u64> {
        use crate::gsf_stat_dictionary::StatDictionary;
        use crate::schema::gsf_ability::decode_gsf_ability_stats;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let dict = StatDictionary::from_embedded()?;
        let conn = self.conn.lock().unwrap();

        let payloads: Vec<(String, Option<String>)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE fqn LIKE 'abl.spvp.%' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, Option<String>)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO gsf_ability_stats \
             (ability_game_id, label, unit, rank, value, confidence, prop_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        let mut count: u64 = 0;
        for (game_id, payload_b64) in &payloads {
            let Some(b64) = payload_b64 else { continue };
            let Ok(payload) = BASE64.decode(b64) else {
                continue;
            };
            let mut rank_per_label: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            for rec in decode_gsf_ability_stats(&payload) {
                let label = dict.ability_label(rec.prop_id);
                let rank = rank_per_label.entry(label.label.clone()).or_insert(0);
                *rank += 1;
                stmt.execute(params![
                    game_id,
                    label.label,
                    label.unit,
                    *rank,
                    rec.value as f64,
                    label.confidence,
                    rec.prop_id as i64,
                ])?;
                count += 1;
            }
        }

        drop(stmt);
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
        let mut align_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO conversation_alignment_events (cnv_fqn, event_kind, event_count) VALUES (?1, ?2, ?3)",
        )?;

        // Alignment-event token kinds. Each entry: (kind_label, byte-needle).
        // Order matters -- prefix patterns (bigdarkmoment) must come before
        // their substring patterns (darkmoment) so the more specific bucket
        // wins.
        let align_needles: &[(&str, &[u8])] = &[
            ("bigdarkmoment", b"event.bigdarkmoment"),
            ("sinistermoment", b"event.sinistermoment"),
            ("darksidetheme", b"event.darksidetheme"),
            ("heroicmoment", b"event.heroicmoment"),
            ("lightsidetheme", b"event.lightsidetheme"),
            ("darkmoment", b"event.darkmoment"),
            ("alignment_override", b"alignment_override"),
            ("influence_desync", b"influence_desync"),
            ("affection_bot", b"affection_bot"),
        ];

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

                // Alignment-event token scan. Walk every printable string in
                // the NODE, count occurrences per kind, write one row per
                // (cnv, kind) with the count. The numbered suffixes
                // (darkmoment_07, heroicmoment_15, ...) collapse into the
                // unsuffixed kind for storage; downstream can re-scan for
                // exact tier numbers if needed.
                let mut align_counts: HashMap<&str, u64> = HashMap::new();
                let mut si = 0;
                while si < data.len() {
                    if (32..127).contains(&data[si]) {
                        let mut sj = si;
                        while sj < data.len() && (32..127).contains(&data[sj]) {
                            sj += 1;
                        }
                        if sj - si >= 5 {
                            let s = &data[si..sj];
                            for (kind, needle) in align_needles {
                                if s.windows(needle.len()).any(|w| w == *needle) {
                                    *align_counts.entry(*kind).or_insert(0) += 1;
                                    break;
                                }
                            }
                        }
                        si = sj;
                    } else {
                        si += 1;
                    }
                }
                for (kind, n) in &align_counts {
                    align_stmt.execute(params![cnv_fqn, kind, n])?;
                    counts.alignment_event += 1;
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
        drop(align_stmt);
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
        let mut stmt = conn.prepare(
            "SELECT fqn, game_id FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
        )?;
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

    /// Populate `quest_clusters` by classifying each quest FQN into one or
    /// more named cluster buckets to support bulk curation.
    ///
    /// A quest can belong to several clusters at different granularities. For
    /// example `qst.location.open_world.imperial.act_1.sith_warrior.legacy`
    /// belongs to: class_act id="imperial|sith_warrior|act_1" plus
    /// class_planet id="open_world|sith_warrior". (The act_N pattern lives
    /// under `open_world`, so the planet bucket carries the literal token
    /// "open_world" rather than a real planet name.)
    pub fn populate_quest_clusters(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT fqn FROM objects WHERE kind = 'Quest' AND is_canonical = 1")?;
        let quest_fqns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        let tx = conn.unchecked_transaction()?;
        let mut ins = tx.prepare_cached(
            "INSERT OR IGNORE INTO quest_clusters (quest_fqn, cluster_kind, cluster_id) \
             VALUES (?1, ?2, ?3)",
        )?;

        let mut count = 0u64;
        for fqn in &quest_fqns {
            for (kind, id) in classify_quest_clusters(fqn) {
                ins.execute(params![fqn, kind, id])?;
                count += 1;
            }
        }
        drop(ins);
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
                "SELECT fqn, game_id FROM objects WHERE fqn LIKE 'qst.location.%.class.%.intro' AND is_canonical = 1",
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
                   AND json_extract(json, '$.strings') IS NOT NULL \
                   AND is_canonical = 1",
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

/// Classify a quest FQN into zero or more (cluster_kind, cluster_id) pairs.
///
/// One quest can populate multiple cluster rows because the FQN encodes
/// orthogonal axes (e.g. a class_act bucket plus an expansion bucket).
fn classify_quest_clusters(fqn: &str) -> Vec<(&'static str, String)> {
    let parts: Vec<&str> = fqn.split('.').collect();
    let mut out: Vec<(&'static str, String)> = Vec::new();

    if parts.len() < 2 || parts[0] != "qst" {
        return out;
    }

    // Pattern: qst.location.open_world.<faction>.act_N.<class>.<rest>
    if parts.len() >= 6
        && parts[1] == "location"
        && parts[2] == "open_world"
        && parts[4].starts_with("act_")
    {
        out.push((
            "class_act",
            format!("{}|{}|{}", parts[3], parts[5], parts[4]),
        ));
    }

    // Pattern: qst.location.<planet>.class.<class>.<rest>
    if parts.len() >= 5 && parts[1] == "location" && parts[3] == "class" {
        out.push(("class_planet", format!("{}|{}", parts[2], parts[4])));
    }

    // Pattern: qst.location.<planet>.world.<faction>.<rest>
    if parts.len() >= 5 && parts[1] == "location" && parts[3] == "world" {
        out.push(("planet_world", format!("{}|{}", parts[2], parts[4])));
    }

    // Pattern: qst.exp.<NN>.<planet>.world_arc.<faction>.hub_N.<rest>
    if parts.len() >= 7
        && parts[1] == "exp"
        && parts[4] == "world_arc"
        && parts[6].starts_with("hub_")
    {
        out.push((
            "world_arc_hub",
            format!("{}|{}|{}|{}", parts[2], parts[3], parts[5], parts[6]),
        ));
        out.push((
            "world_arc",
            format!("{}|{}|{}", parts[2], parts[3], parts[5]),
        ));
    } else if parts.len() >= 4 && parts[1] == "exp" {
        // Generic expansion bucket: qst.exp.<NN>.<planet|seg>.<rest>
        out.push((
            "expansion_arc",
            format!("{}|{}", parts[2], parts.get(3).copied().unwrap_or("")),
        ));
    }

    // Pattern: qst.daily_area.<planet>.<rest>
    if parts.len() >= 3 && parts[1] == "daily_area" {
        out.push(("daily_area", parts[2].to_string()));
    }

    // Pattern: qst.heroic.<planet_or_name>.<rest>
    if parts.len() >= 3 && parts[1] == "heroic" {
        out.push(("heroic", parts[2].to_string()));
    }

    // Pattern: qst.flashpoint.<name>.<rest>
    if parts.len() >= 3 && parts[1] == "flashpoint" {
        out.push(("flashpoint", parts[2].to_string()));
    }

    // Pattern: qst.operation.<name>.<rest>
    if parts.len() >= 3 && parts[1] == "operation" {
        out.push(("operation", parts[2].to_string()));
    }

    // Pattern: qst.event.<event_name>.<rest>
    if parts.len() >= 3 && parts[1] == "event" {
        out.push(("event", parts[2].to_string()));
    }

    // Pattern: qst.alliance.companion.<class>.<rest>
    if parts.len() >= 4 && parts[1] == "alliance" && parts[2] == "companion" {
        out.push(("companion", parts[3].to_string()));
    } else if parts.len() >= 3 && parts[1] == "alliance" {
        out.push(("alliance", parts[2].to_string()));
    }

    // Pattern: qst.qtr.<rest>
    if parts.len() >= 2 && parts[1] == "qtr" {
        let leaf = parts.get(2).copied().unwrap_or("");
        out.push(("qtr", leaf.to_string()));
    }

    // Pattern: qst.ventures.<rest>
    if parts.len() >= 2 && parts[1] == "ventures" {
        let leaf = parts.get(2).copied().unwrap_or("");
        out.push(("ventures", leaf.to_string()));
    }

    // Galactic seasons: qst.exp.galactic_seasons.<season>.*
    if parts.len() >= 4 && parts[1] == "exp" && parts[2] == "galactic_seasons" {
        out.push(("galactic_seasons", parts[3].to_string()));
    }
    // Or qst.event.galactic_seasons.<season>.*
    if parts.len() >= 4 && parts[1] == "event" && parts[2] == "galactic_seasons" {
        out.push(("galactic_seasons", parts[3].to_string()));
    }

    out
}

/// Parse a conquest objective FQN (`ach.conquests.<category>.<sub>...<leaf>`)
/// into (category, subcategory, cadence). Cadence is `Some("weekly")` if the
/// leaf ends with `_weekly` or path contains `.weekly.`, `Some("daily")` if
/// the path contains `.daily.`, otherwise `None` for repeatable objectives.
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
        .prepare("SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind = ?1 AND is_canonical = 1")?;
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

            let mut qst_stmt =
                conn.prepare("SELECT fqn FROM objects WHERE kind = 'Quest' AND is_canonical = 1")?;
            let qst_fqns: Vec<String> = qst_stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(qst_stmt);

            let mut phase_stmt =
                conn.prepare("SELECT fqn FROM objects WHERE kind = 'Phase' AND is_canonical = 1")?;
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
        //
        // Special case: don't collapse `stage_<N>` leaves. Multi-stage bonus
        // missions (e.g. `mpn.X.bonus.Y.staged.Z.stage_2`) should keep each
        // stage as its own mission, since human-curated checklists count
        // each stage independently.
        let mut mpn_prefixes: HashSet<String> = HashSet::new();
        for phase in &phase_fqns {
            let Some(last_dot) = phase.rfind('.') else {
                continue;
            };
            let leaf = &phase[last_dot + 1..];
            let prefix = if leaf.starts_with("stage_") {
                phase.as_str()
            } else {
                &phase[..last_dot]
            };
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

        // Achievement-as-mission rows. SWTOR encodes some checklist content
        // as `ach.*` instead of qst/mpn -- galactic seasons priority
        // objectives, dynamic-event objectives, ventures progression,
        // conquests. Add those as mission rows with source naming the
        // achievement family.
        let ach_fqns: Vec<String> = {
            let mut stmt2 = tx.prepare(
                "SELECT fqn FROM objects WHERE kind = 'Achievement' \
                 AND is_canonical = 1 \
                 AND ( \
                    fqn LIKE 'ach.galactic_seasons.season_%' \
                    OR fqn LIKE 'ach.dynamic_events.%' \
                    OR fqn LIKE 'ach.ventures.%' \
                    OR fqn LIKE 'ach.conquests.%' \
                 )",
            )?;
            let collected: Vec<String> = stmt2
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };
        for fqn in &ach_fqns {
            let source = if fqn.starts_with("ach.galactic_seasons.") {
                "achievement_gs"
            } else if fqn.starts_with("ach.dynamic_events.") {
                "achievement_dynamic"
            } else if fqn.starts_with("ach.ventures.") {
                "achievement_ventures"
            } else {
                "conquest"
            };
            stmt.execute(rusqlite::params![fqn, source])?;
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
                "SELECT fqn, string_id FROM objects WHERE kind = 'Achievement' AND fqn LIKE 'ach.conquests.%' AND is_canonical = 1",
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
                "SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
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
                 WHERE kind IN ('Npc', 'Spawn', 'Encounter') AND is_canonical = 1",
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
    ///   abl.{class}.{name}                                 -> CLASS-SHARED, fanned out to every discipline of {class}
    ///                                                        (e.g. abl.jedi_knight.saber_reflect, abl.jedi_knight.force_leap)
    pub fn populate_disciplines(&self) -> Result<(u64, u64, u64)> {
        self.flush()?;

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, game_id FROM objects \
                 WHERE fqn LIKE 'abl.%' AND kind = 'Ability' AND is_canonical = 1",
            )?;
            let result: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

        let mut disc_set: std::collections::HashSet<(String, String, String, String)> =
            std::collections::HashSet::new(); // (origin, discipline, fqn_prefix, combat_style)
        let mut abl_rows: Vec<(String, String, String, Option<u8>, String)> = Vec::new();
        let mut class_shared: Vec<(String, String, String)> = Vec::new();
        let mut utility_shared: Vec<(String, String, String)> = Vec::new();
        let mut shared_mod: Vec<(String, String, String)> = Vec::new();

        for (fqn, game_id) in &rows {
            let parts: Vec<&str> = fqn.split('.').collect();

            // abl.<origin>.<name>: 3-segment class-shared ability (Force
            // Leap, Saber Reflect, etc). Per-origin pool, fanned to BOTH
            // combat styles in combat_style_shared_abilities below.
            if parts.len() == 3 {
                let origin = parts[1];
                if PLAYER_ORIGINS.contains(&origin) {
                    class_shared.push((origin.to_string(), fqn.clone(), game_id.clone()));
                }
                continue;
            }

            if parts.len() < 5 || parts[2] != "skill" {
                continue;
            }
            let origin = parts[1];
            if !PLAYER_ORIGINS.contains(&origin) {
                continue;
            }

            // Per-origin shared pools: route to combat_style_shared_abilities,
            // not into disciplines. The disciplines table now holds only
            // real combat disciplines (24 rows: 16 styles x 1.5 avg, post
            // 7.0 reductions).
            if parts[3] == "utility" {
                utility_shared.push((origin.to_string(), fqn.clone(), game_id.clone()));
                continue;
            }
            if parts[3] == "mods" {
                shared_mod.push((origin.to_string(), fqn.clone(), game_id.clone()));
                continue;
            }

            // abl.<origin>.skill.<discipline>.*
            let discipline_name = parts[3];
            let fqn_prefix = format!("abl.{}.skill.{}", origin, discipline_name);
            let Some(combat_style) = combat_style_for(origin, discipline_name) else {
                // Unknown discipline -- skip rather than emit a NULL
                // combat_style_codename. If a new SWTOR patch adds one,
                // it'll show up here as a missing extraction and force
                // an explicit map update.
                continue;
            };

            let slot_type: &str;
            let tier_level: Option<u8>;
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

            disc_set.insert((
                origin.to_string(),
                discipline_name.to_string(),
                fqn_prefix.clone(),
                combat_style.to_string(),
            ));
            abl_rows.push((
                fqn_prefix,
                game_id.clone(),
                fqn.clone(),
                tier_level,
                slot_type.to_string(),
            ));
        }

        // Fan per-origin shared/utility/shared_mod abilities to BOTH combat
        // styles of that origin -- e.g. Force Leap (sith_warrior class-shared)
        // emits one row for juggernaut and one for marauder.
        let mut shared_rows: Vec<(String, String, String, String)> = Vec::new();
        for (origin, ability_fqn, game_id) in &class_shared {
            for combat_style in origin_combat_styles(origin) {
                shared_rows.push((
                    combat_style.to_string(),
                    game_id.clone(),
                    ability_fqn.clone(),
                    "class_shared".to_string(),
                ));
            }
        }
        for (origin, ability_fqn, game_id) in &utility_shared {
            for combat_style in origin_combat_styles(origin) {
                shared_rows.push((
                    combat_style.to_string(),
                    game_id.clone(),
                    ability_fqn.clone(),
                    "utility".to_string(),
                ));
            }
        }
        for (origin, ability_fqn, game_id) in &shared_mod {
            for combat_style in origin_combat_styles(origin) {
                shared_rows.push((
                    combat_style.to_string(),
                    game_id.clone(),
                    ability_fqn.clone(),
                    "shared_mod".to_string(),
                ));
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut disc_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO disciplines (origin_codename, discipline_name, fqn_prefix, combat_style_codename) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (origin, discipline_name, fqn_prefix, combat_style) in &disc_set {
                stmt.execute(params![origin, discipline_name, fqn_prefix, combat_style])?;
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

        let mut shared_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO combat_style_shared_abilities \
                   (combat_style_codename, ability_game_id, ability_fqn, slot_type) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (combat_style, game_id, fqn, slot) in &shared_rows {
                stmt.execute(params![combat_style, game_id, fqn, slot])?;
                shared_count += 1;
            }
        }

        tx.commit()?;
        Ok((disc_count, abl_count, shared_count))
    }

    /// Populate `discipline_talents` by FQN pattern.
    ///
    /// Mechanical mapping:
    /// - `tal.<class>.skill.<discipline>.<rest>` belongs to that one
    ///   combat discipline (`abl.<class>.skill.<discipline>`).
    /// - `tal.<class>.skill.utility.<rest>` is a per-class SHARED utility
    ///   talent. Fanned out: one row per combat discipline of that class,
    ///   plus its own row under the utility discipline. This mirrors how
    ///   SWTOR exposes utility talents in the discipline UI -- a
    ///   utility-talent pick is visible on every combat-tree page for that
    ///   class.
    ///
    /// No tier_level / column coordinates: SWTOR's editorial tree layout
    /// (which talent sits at what tier on screen) is not encoded in tal.*
    /// payloads or FQNs. This function only emits mechanical membership.
    pub fn populate_discipline_talents(&self) -> Result<(u64, u64)> {
        self.flush()?;

        let talents: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, game_id FROM objects \
                 WHERE kind = 'Talent' AND fqn LIKE 'tal.%.skill.%' AND is_canonical = 1",
            )?;
            let result: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            result
        };

        let mut discipline_rows: Vec<(String, String, String)> = Vec::new();
        let mut utility_rows: Vec<(String, String, String)> = Vec::new(); // (combat_style, talent_game_id, talent_fqn)

        for (fqn, game_id) in &talents {
            // tal.<origin>.skill.<segment>.<rest>
            let parts: Vec<&str> = fqn.split('.').collect();
            if parts.len() < 5 || parts[2] != "skill" {
                continue;
            }
            let origin = parts[1];
            let segment = parts[3];
            if !PLAYER_ORIGINS.contains(&origin) {
                continue;
            }

            if segment == "utility" {
                // Per-origin utility talent: fan to BOTH combat styles. No
                // discipline_talents row -- utility is not a real discipline.
                for combat_style in origin_combat_styles(origin) {
                    utility_rows.push((combat_style.to_string(), game_id.clone(), fqn.clone()));
                }
                continue;
            }

            // Discipline-specific talent: literal-membership row only, no
            // fan-out (the previous fan-out was the bug we're fixing).
            let literal_prefix = format!("abl.{}.skill.{}", origin, segment);
            discipline_rows.push((literal_prefix, game_id.clone(), fqn.clone()));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut disc_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO discipline_talents \
                 (discipline_fqn_prefix, talent_game_id, talent_fqn) VALUES (?1, ?2, ?3)",
            )?;
            for (disc, game_id, fqn) in &discipline_rows {
                stmt.execute(params![disc, game_id, fqn])?;
                disc_count += 1;
            }
        }

        let mut util_count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO class_utility_talents \
                 (combat_style_codename, talent_game_id, talent_fqn) VALUES (?1, ?2, ?3)",
            )?;
            for (combat_style, game_id, fqn) in &utility_rows {
                stmt.execute(params![combat_style, game_id, fqn])?;
                util_count += 1;
            }
        }

        tx.commit()?;
        Ok((disc_count, util_count))
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
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn, json FROM objects WHERE kind = 'Talent' AND is_canonical = 1",
            )?;
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

    /// Populate `origins` (#94). 8 rows, hardcoded from canonical PLAYER_CLASSES.
    /// Faction + attack_type are intrinsic to each origin and don't change with
    /// patches. `string_id` is left NULL until the canonical name source is
    /// confirmed (see follow-up on #94).
    pub fn populate_origins(&self) -> Result<u64> {
        self.flush()?;

        // (fqn_segment, faction, attack_type)
        const ORIGINS: &[(&str, &str, &str)] = &[
            ("sith_warrior", "empire", "force"),
            ("sith_inquisitor", "empire", "force"),
            ("bounty_hunter", "empire", "tech"),
            ("agent", "empire", "tech"),
            ("jedi_knight", "republic", "force"),
            ("jedi_consular", "republic", "force"),
            ("trooper", "republic", "tech"),
            ("smuggler", "republic", "tech"),
        ];

        // Resolve canonical display name via `cdx.game_rules.classes.<seg>`
        // -- a clean sequential block of 8 codex strings whose FQN trailing
        // segment matches our origin codenames exactly (agent, not
        // imperial_agent; sith_inquisitor, not inquisitor). Sample row:
        //   cdx.game_rules.classes.sith_warrior -> string_id 571462 -> "Sith Warrior".
        let cdx_strings: std::collections::HashMap<String, i64> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, string_id FROM objects \
                 WHERE fqn LIKE 'cdx.game_rules.classes.%' AND is_canonical = 1 \
                   AND string_id IS NOT NULL",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter()
                .filter_map(|(fqn, sid)| {
                    fqn.strip_prefix("cdx.game_rules.classes.")
                        .map(|seg| (seg.to_string(), sid))
                })
                .collect()
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO origins (fqn_segment, faction, attack_type, string_id) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (seg, faction, atk) in ORIGINS {
                let sid = cdx_strings.get(*seg).copied();
                stmt.execute(params![seg, faction, atk, sid])?;
            }
        }
        tx.commit()?;
        Ok(ORIGINS.len() as u64)
    }

    /// Populate `combat_styles` (#94) from `class.pc.advanced.*` GOM objects.
    /// Resolves the localized display name via `cdx.advanced_classes.<display>`,
    /// where `<display>` is the canonical name (matches the internal segment for
    /// most styles, but `force_wizard` -> `jedi_sage` and `specialist` ->
    /// `vanguard`). Some other internal names map to prefixed cdx names:
    /// `assassin` -> `sith_assassin`, `shadow` -> `jedi_shadow`, `sorcerer` ->
    /// `sith_sorcerer`.
    pub fn populate_combat_styles(&self) -> Result<u64> {
        self.flush()?;

        // Internal segment -> canonical display segment used by cdx.advanced_classes.*.
        // Default mapping is identity; this table covers only the divergent ones.
        fn display_for(seg: &str) -> &str {
            match seg {
                "force_wizard" => "jedi_sage",
                "specialist" => "vanguard",
                "assassin" => "sith_assassin",
                "shadow" => "jedi_shadow",
                "sorcerer" => "sith_sorcerer",
                other => other,
            }
        }

        // Force-vs-tech split. Cleaner here than in the cdx pass since
        // pre-7.0 advanced-class identity is what determines pool, not
        // the codex name.
        fn attack_type(seg: &str) -> &'static str {
            match seg {
                "assassin" | "shadow" | "sorcerer" | "force_wizard" | "juggernaut" | "guardian"
                | "marauder" | "sentinel" => "force",
                "powertech" | "vanguard" | "specialist" | "mercenary" | "commando"
                | "operative" | "scoundrel" | "sniper" | "gunslinger" => "tech",
                _ => "unknown",
            }
        }

        // Legacy adv-class faction. Post-7.0 origin/style decoupling does not
        // change which faction's silhouette/animations a style ships with --
        // juggernauts are still empire-coded, guardians republic-coded.
        // huttspawn nav and color tokens are strictly bipartite on this.
        fn faction(seg: &str) -> &'static str {
            match seg {
                "juggernaut" | "marauder" | "assassin" | "sorcerer" | "powertech" | "mercenary"
                | "operative" | "sniper" => "empire",
                "guardian" | "sentinel" | "shadow" | "force_wizard" | "specialist" | "commando"
                | "scoundrel" | "gunslinger" => "republic",
                _ => "unknown",
            }
        }

        // Load class.pc.advanced.* canonical rows (the 16 combat-style objects).
        let combat_objects: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn FROM objects \
                 WHERE fqn LIKE 'class.pc.advanced.%' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        // Load cdx.advanced_classes.<display> -> string_id map.
        let cdx_strings: std::collections::HashMap<String, i64> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, string_id FROM objects \
                 WHERE fqn LIKE 'cdx.advanced_classes.%' AND is_canonical = 1 \
                   AND string_id IS NOT NULL",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter()
                .filter_map(|(fqn, sid)| {
                    fqn.strip_prefix("cdx.advanced_classes.")
                        .map(|seg| (seg.to_string(), sid))
                })
                .collect()
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO combat_styles \
                   (combat_style_id, fqn, fqn_segment, display_segment, faction, attack_type, string_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (game_id, fqn) in &combat_objects {
                let Some(seg) = fqn.strip_prefix("class.pc.advanced.") else {
                    continue;
                };
                let display = display_for(seg);
                let atk = attack_type(seg);
                let fac = faction(seg);
                let sid = cdx_strings.get(display).copied();
                stmt.execute(params![game_id, fqn, seg, display, fac, atk, sid])?;
                count += 1;
            }
        }
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
        let origins: u64 = conn.query_row("SELECT COUNT(*) FROM origins", [], |row| row.get(0))?;
        let combat_styles: u64 =
            conn.query_row("SELECT COUNT(*) FROM combat_styles", [], |row| row.get(0))?;
        let combat_style_shared_abilities: u64 = conn.query_row(
            "SELECT COUNT(*) FROM combat_style_shared_abilities",
            [],
            |row| row.get(0),
        )?;
        let class_utility_talents: u64 =
            conn.query_row("SELECT COUNT(*) FROM class_utility_talents", [], |row| {
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
            origins,
            combat_styles,
            combat_style_shared_abilities,
            class_utility_talents,
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
            .prepare("SELECT icon_name, game_id, kind FROM objects WHERE icon_name IS NOT NULL AND is_canonical = 1")?;

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
        let mut stmt = conn.prepare(
            "SELECT fqn, game_id, kind FROM objects WHERE icon_name IS NULL AND is_canonical = 1",
        )?;

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
/// Convert a snake_case FQN segment to a title-cased display name.
/// e.g. `mag_bolt` -> `Mag Bolt`, `fueled_corruption` -> `Fueled Corruption`.
/// Used by `backfill_missing_string_ids` to derive a candidate display name
/// when the GOM payload lacks a string-table marker.
fn title_case_from_snake(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Origin codenames -- the second FQN segment in `abl.<origin>.skill.*`.
/// 8 player origins (sith_warrior, agent, etc.). Other `abl.<X>.<name>` FQNs
/// (companion, racials, legacy, location, customer_service, ...) aren't
/// player-discipline content; we ignore them.
const PLAYER_ORIGINS: &[&str] = &[
    "agent",
    "bounty_hunter",
    "jedi_consular",
    "jedi_knight",
    "sith_inquisitor",
    "sith_warrior",
    "smuggler",
    "trooper",
];

/// Origin -> 2 combat styles. Used to fan per-origin shared/utility pools
/// (abl.<origin>.<name>, abl.<origin>.skill.utility.*, abl.<origin>.skill.mods.*,
/// tal.<origin>.skill.utility.*) into combat_style_shared_abilities and
/// class_utility_talents.
fn origin_combat_styles(origin: &str) -> &'static [&'static str] {
    match origin {
        "sith_warrior" => &["juggernaut", "marauder"],
        "sith_inquisitor" => &["assassin", "sorcerer"],
        "bounty_hunter" => &["powertech", "mercenary"],
        "agent" => &["operative", "sniper"],
        "jedi_knight" => &["guardian", "sentinel"],
        "jedi_consular" => &["shadow", "force_wizard"],
        "trooper" => &["specialist", "commando"],
        "smuggler" => &["scoundrel", "gunslinger"],
        _ => &[],
    }
}

/// (origin, discipline_name) -> combat_style_codename.
/// 48-row map (8 origins * 6 disciplines). Stable since SWTOR 4.0; new
/// disciplines force an explicit update via populate_disciplines's
/// "unknown discipline -> skip" branch.
///
/// Names use SOURCE-data canon (firebug not pyrotech, combat not kinetic_combat).
/// huttspawn ETL renames at the editorial layer per #51.
/// 48-row table: every (origin, discipline) pair maps to exactly one combat
/// style. Stored as a flat constant so unit tests can iterate it and assert
/// invariants (combat-style values join `combat_styles.fqn_segment`, every
/// combat style appears in `origin_combat_styles`, etc).
const DISCIPLINE_COMBAT_STYLE_MAP: &[(&str, &str, &str)] = &[
    ("sith_warrior", "annihilation", "marauder"),
    ("sith_warrior", "carnage", "marauder"),
    ("sith_warrior", "fury", "marauder"),
    ("sith_warrior", "immortal", "juggernaut"),
    ("sith_warrior", "rage", "juggernaut"),
    ("sith_warrior", "vengeance", "juggernaut"),
    ("sith_inquisitor", "darkness", "assassin"),
    ("sith_inquisitor", "deception", "assassin"),
    ("sith_inquisitor", "hatred", "assassin"),
    ("sith_inquisitor", "corruption", "sorcerer"),
    ("sith_inquisitor", "lightning", "sorcerer"),
    ("sith_inquisitor", "madness", "sorcerer"),
    ("bounty_hunter", "advanced_prototype", "powertech"),
    ("bounty_hunter", "firebug", "powertech"),
    ("bounty_hunter", "shield_tech", "powertech"),
    ("bounty_hunter", "arsenal", "mercenary"),
    ("bounty_hunter", "bodyguard", "mercenary"),
    ("bounty_hunter", "innovative_ordnance", "mercenary"),
    ("agent", "concealment", "operative"),
    ("agent", "lethality", "operative"),
    ("agent", "medic", "operative"),
    ("agent", "engineering", "sniper"),
    ("agent", "marksmanship", "sniper"),
    ("agent", "virulence", "sniper"),
    ("jedi_knight", "defense", "guardian"),
    ("jedi_knight", "focus", "guardian"),
    ("jedi_knight", "vigilance", "guardian"),
    ("jedi_knight", "combat", "sentinel"),
    ("jedi_knight", "concentration", "sentinel"),
    ("jedi_knight", "watchman", "sentinel"),
    ("jedi_consular", "balance", "force_wizard"),
    ("jedi_consular", "seer", "force_wizard"),
    ("jedi_consular", "telekinetics", "force_wizard"),
    ("jedi_consular", "combat", "shadow"),
    ("jedi_consular", "infiltration", "shadow"),
    ("jedi_consular", "serenity", "shadow"),
    ("trooper", "assault_specialist", "commando"),
    ("trooper", "combat_medic", "commando"),
    ("trooper", "gunnery", "commando"),
    ("trooper", "plasmatech", "specialist"),
    ("trooper", "shield_specialist", "specialist"),
    ("trooper", "tactics", "specialist"),
    ("smuggler", "ruffian", "scoundrel"),
    ("smuggler", "sawbones", "scoundrel"),
    ("smuggler", "scrapper", "scoundrel"),
    ("smuggler", "dirty_fighting", "gunslinger"),
    ("smuggler", "saboteur", "gunslinger"),
    ("smuggler", "sharpshooter", "gunslinger"),
];

fn combat_style_for(origin: &str, discipline: &str) -> Option<&'static str> {
    DISCIPLINE_COMBAT_STYLE_MAP
        .iter()
        .find(|(o, d, _)| *o == origin && *d == discipline)
        .map(|(_, _, cs)| *cs)
}

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

/// Decoded ability properties from a single sentinel-anchored prop block.
#[derive(Default)]
struct AbilityStats {
    cooldown: Option<f32>,
    cast_time: Option<f32>,
    channel_duration: Option<f32>,
    hard_cast_time: Option<f32>,
    force_cost: Option<i32>,
    resource_cost: Option<i32>,
    raw_props_json: Option<String>,
}

impl AbilityStats {
    fn any_hit(&self) -> bool {
        self.cooldown.is_some()
            || self.cast_time.is_some()
            || self.channel_duration.is_some()
            || self.hard_cast_time.is_some()
            || self.force_cost.is_some()
            || self.resource_cost.is_some()
            || self.raw_props_json.is_some()
    }
}

const ABILITY_PROP_SENTINEL: [u8; 6] = [0x01, 0x04, 0x00, 0x00, 0x80, 0xBF];

/// Map an `abl.*` or `tal.*` FQN to a normalized resource pool / category tag.
///
/// Player class FQNs resolve to their resource pool (rage/focus/force/heat/
/// ammo/energy). Galactic Starfighter (`*.spvp.*`) resolves to `gsf` — GSF
/// uses a 3-pool blaster/engine/shield system, the tag identifies the game
/// mode. On-rails Space Combat (`*.space_combat.*`) and companion / racial /
/// legacy / spvp-buff entries resolve to None.
fn resource_pool_from_fqn(fqn: &str) -> Option<&'static str> {
    let segments: Vec<&str> = fqn.split('.').collect();
    if segments.len() < 2 {
        return None;
    }
    if segments[0] != "abl" && segments[0] != "tal" {
        return None;
    }
    match segments[1] {
        "sith_warrior" => Some("rage"),
        "jedi_knight" => Some("focus"),
        "sith_inquisitor" | "jedi_consular" => Some("force"),
        "bounty_hunter" => Some("heat"),
        "trooper" => Some("ammo"),
        "agent" | "smuggler" => Some("energy"),
        "spvp" => Some("gsf"),
        _ => None,
    }
}

/// Length-prefixed ASCII tail string at the end of a talent payload (vault
/// MAPPINGS.md lines 339-365). The byte at `payload[i]` is the length, the
/// `payload[i+1..i+1+len]` bytes are the ASCII script-hook identifier. Walks
/// backward from end of payload to find the most recent valid pattern, since
/// hooks live at the tail. Returns the decoded string, or None if no
/// plausible candidate is found.
fn extract_talent_script_hook(payload: &[u8]) -> Option<String> {
    let n = payload.len();
    // Walk back through the last 96 bytes looking for a length prefix that
    // points to a printable ASCII identifier extending to the end.
    let max_lookback = n.min(96);
    for offset in 1..max_lookback {
        let i = n.saturating_sub(offset + 1);
        let len_byte = payload[i] as usize;
        if !(4..=80).contains(&len_byte) {
            continue;
        }
        let start = i + 1;
        let end = start + len_byte;
        if end > n {
            continue;
        }
        let chunk = &payload[start..end];
        if !chunk.iter().all(|b| (32..127).contains(b)) {
            continue;
        }
        let s = std::str::from_utf8(chunk).ok()?;
        // Hook identifiers are alphanumeric + underscore, must start alpha.
        let bytes = s.as_bytes();
        if !bytes[0].is_ascii_alphabetic() {
            continue;
        }
        if !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            continue;
        }
        return Some(s.to_string());
    }
    None
}

/// Walk the sentinel-anchored ability property block. Finds the first
/// occurrence of `01 04 00 00 80 BF` (= 0x0401 with -1.0 uninit value), then
/// reads contiguous 6-byte `[u16 LE prop_id][f32 LE value]` records until the
/// next 2 bytes are not in the 0x04xx range. Returns an empty result for
/// abilities on the secondary template that have no such block.
///
/// Verified field-ID semantics (template 400000000283F4D2):
///   0x0401 = cooldown seconds
///   0x041b = cast / channel time seconds
///   0x0406 = channel duration seconds
///   0x041a = alternate hard cast time
///   0x0403 = Force-pool cost (sorcerer/sage; tech writes a low scaling
///            coefficient here, so values < 5 are dropped as non-cost)
///   0x041e = heat / energy / ammo cost (tech)
///
/// Other in-block fields (0x041d, 0x041f, 0x0420, 0x0421, 0x0402, 0x0404)
/// have class-context-dependent semantics that do not map cleanly to flat
/// columns; they land in `raw_props` JSON for follow-up analysis.
fn scan_ability_props(payload: &[u8]) -> AbilityStats {
    use std::collections::BTreeMap;

    let mut stats = AbilityStats::default();
    let Some(start) = find_subslice(payload, &ABILITY_PROP_SENTINEL) else {
        return stats;
    };

    let mut raw: BTreeMap<u16, f32> = BTreeMap::new();
    let mut i = start;
    while i + 6 <= payload.len() {
        let prop_id = u16::from_le_bytes([payload[i], payload[i + 1]]);
        if (prop_id >> 8) != 0x04 {
            break;
        }
        let value = f32::from_le_bytes([
            payload[i + 2],
            payload[i + 3],
            payload[i + 4],
            payload[i + 5],
        ]);
        if value.is_finite() {
            // The sentinel itself (0x0401 = -1.0) should not displace later
            // 0x0401 records, which carry the actual cooldown.
            let is_sentinel = prop_id == 0x0401 && value == -1.0;
            if !is_sentinel {
                raw.entry(prop_id).or_insert(value);
                match prop_id {
                    0x0401 if stats.cooldown.is_none() => stats.cooldown = Some(value),
                    0x041b if stats.cast_time.is_none() => stats.cast_time = Some(value),
                    0x0406 if stats.channel_duration.is_none() => {
                        stats.channel_duration = Some(value)
                    }
                    0x041a if stats.hard_cast_time.is_none() => stats.hard_cast_time = Some(value),
                    0x0403 if stats.force_cost.is_none() && value >= 5.0 => {
                        stats.force_cost = Some(value as i32)
                    }
                    0x041e if stats.resource_cost.is_none() && value >= 1.0 => {
                        stats.resource_cost = Some(value as i32)
                    }
                    _ => {}
                }
            }
        }
        i += 6;
    }

    if !raw.is_empty() {
        // Re-key raw_props from hex (`0x041f`) to plain labels
        // (`melee_range_meters`) via the embedded dictionary so consumers
        // can read the JSON without a hex lookup table. Unknown prop IDs
        // get a synthesised `unknown_0x<id>` key.
        let dict = crate::gsf_stat_dictionary::StatDictionary::from_embedded()
            .expect("embedded gsf_stat_dictionary.toml parses");
        let map: serde_json::Map<String, serde_json::Value> = raw
            .into_iter()
            .map(|(id, val)| {
                let label = dict.ground_ability_label(id);
                (label.label, serde_json::json!(val))
            })
            .collect();
        stats.raw_props_json = serde_json::to_string(&map).ok();
    }
    stats
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kessel_test_{}_{}_{}.sqlite", label, pid, nanos))
    }

    #[test]
    fn populate_origins_inserts_eight_canonical_rows() {
        let path = temp_db_path("origins");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Seed canonical-name codex rows for two origins -- the populator
        // should pick these up via cdx.game_rules.classes.<seg>. Other six
        // origins have no codex row in this fixture; their string_id should
        // remain NULL (graceful missing-source behaviour).
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('og_sw', 'sid1', 'ph1', 'guid1', 'cdx.game_rules.classes.sith_warrior', 'Codex', 571462, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('og_ag', 'sid2', 'ph2', 'guid2', 'cdx.game_rules.classes.agent', 'Codex', 571473, '{}')",
                [],
            ).unwrap();
        }

        let count = db.populate_origins().unwrap();
        assert_eq!(count, 8);

        let conn = db.conn.lock().unwrap();
        let total: u64 = conn
            .query_row("SELECT COUNT(*) FROM origins", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 8);

        // Faction split: 4/4 between empire and republic.
        let empire: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM origins WHERE faction = 'empire'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(empire, 4);

        // Attack-type split: 4/4 force vs tech, exactly the eligibility pools.
        let force: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM origins WHERE attack_type = 'force'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(force, 4);

        // Sith warrior must be empire+force; trooper must be republic+tech.
        let (sw_faction, sw_atk): (String, String) = conn
            .query_row(
                "SELECT faction, attack_type FROM origins WHERE fqn_segment = 'sith_warrior'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(sw_faction, "empire");
        assert_eq!(sw_atk, "force");

        let (tr_faction, tr_atk): (String, String) = conn
            .query_row(
                "SELECT faction, attack_type FROM origins WHERE fqn_segment = 'trooper'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tr_faction, "republic");
        assert_eq!(tr_atk, "tech");

        // Seeded origins resolve string_id; unseeded origins stay NULL.
        let sw_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'sith_warrior'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sw_sid, Some(571462));

        let ag_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'agent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ag_sid, Some(571473));

        let tr_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'trooper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tr_sid, None);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn populate_combat_styles_resolves_codename_and_string_id() {
        let path = temp_db_path("combat_styles");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Seed a class.pc.advanced.* internal-codename row and the matching
        // cdx.advanced_classes.* display row with a string_id.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
                 VALUES ('cs_force_wizard', 'sid1', 'ph1', 'guid1', 'class.pc.advanced.force_wizard', 'class', '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('cdx_sage', 'sid2', 'ph2', 'guid2', 'cdx.advanced_classes.jedi_sage', 'Codex', 351322, '{}')",
                [],
            ).unwrap();
            // A tech style with identity-mapped name + cdx string.
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
                 VALUES ('cs_powertech', 'sid3', 'ph3', 'guid3', 'class.pc.advanced.powertech', 'class', '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('cdx_pt', 'sid4', 'ph4', 'guid4', 'cdx.advanced_classes.powertech', 'Codex', 351335, '{}')",
                [],
            ).unwrap();
        }

        let count = db.populate_combat_styles().unwrap();
        assert_eq!(count, 2);

        let conn = db.conn.lock().unwrap();

        // force_wizard codename resolves to display 'jedi_sage' and gets sage's string_id.
        let (display, fac, atk, sid): (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT display_segment, faction, attack_type, string_id FROM combat_styles \
                 WHERE fqn_segment = 'force_wizard'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(display, "jedi_sage");
        assert_eq!(fac, "republic");
        assert_eq!(atk, "force");
        assert_eq!(sid, Some(351322));

        // Identity-mapped style still gets its string_id.
        let (pt_display, pt_fac, pt_atk, pt_sid): (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT display_segment, faction, attack_type, string_id FROM combat_styles \
                 WHERE fqn_segment = 'powertech'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(pt_display, "powertech");
        assert_eq!(pt_fac, "empire");
        assert_eq!(pt_atk, "tech");
        assert_eq!(pt_sid, Some(351335));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// Helper: insert an Ability or Talent object so populate_disciplines and
    /// populate_discipline_talents have something to find.
    fn insert_obj(db: &Database, game_id: &str, fqn: &str, kind: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
             VALUES (?1, 'sid', 'ph', 'guid', ?2, ?3, '{}')",
            params![game_id, fqn, kind],
        )
        .unwrap();
    }

    /// Helper: seed `combat_styles` rows for a given origin so disciplines/css/cut
    /// inserts can satisfy their FK to combat_styles(fqn_segment).
    fn seed_combat_styles_for(db: &Database, origin: &str) {
        let conn = db.conn.lock().unwrap();
        for cs in origin_combat_styles(origin) {
            let game_id = format!("cs_{}", cs);
            let fqn = format!("class.pc.advanced.{}", cs);
            conn.execute(
                "INSERT OR IGNORE INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'class', '{}')",
                params![game_id, format!("sid_{}", cs), format!("ph_{}", cs), format!("guid_{}", cs), fqn],
            )
            .unwrap();
            conn.execute(
                "INSERT OR IGNORE INTO combat_styles \
                   (combat_style_id, fqn, fqn_segment, display_segment, faction, attack_type) \
                 VALUES (?1, ?2, ?3, ?3, 'unknown', 'unknown')",
                params![game_id, fqn, cs],
            )
            .unwrap();
        }
    }

    #[test]
    fn populate_disciplines_emits_combat_style_codename_and_routes_shared_pool() {
        let path = temp_db_path("disc_rework");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        seed_combat_styles_for(&db, "sith_warrior");

        // Combat-discipline ability for sith_warrior (vengeance is a juggernaut spec).
        insert_obj(
            &db,
            "v1",
            "abl.sith_warrior.skill.vengeance.ravage",
            "Ability",
        );
        // Class-shared ability (3-segment FQN) -- should fan to BOTH combat styles.
        insert_obj(&db, "fl", "abl.sith_warrior.force_leap", "Ability");
        // Skill-utility ability -- per-origin pool, fans to both styles.
        insert_obj(
            &db,
            "th",
            "abl.sith_warrior.skill.utility.thwart",
            "Ability",
        );
        // Shared-mod ability -- per-origin pool, fans to both styles.
        insert_obj(
            &db,
            "sm",
            "abl.sith_warrior.skill.mods.tier1.savagery",
            "Ability",
        );
        // Non-player-origin abl.* should be ignored entirely.
        insert_obj(&db, "co", "abl.companion.attack", "Ability");

        let (disc_count, disc_abl_count, css_abl_count) = db.populate_disciplines().unwrap();

        // One discipline (vengeance), one discipline_abilities row (ravage).
        assert_eq!(disc_count, 1);
        assert_eq!(disc_abl_count, 1);
        // Three shared abilities (force_leap, thwart, savagery) x 2 combat styles = 6 rows.
        assert_eq!(css_abl_count, 6);

        let conn = db.conn.lock().unwrap();

        // disciplines: combat_style_codename resolved correctly.
        let (origin, combat_style): (String, String) = conn
            .query_row(
                "SELECT origin_codename, combat_style_codename FROM disciplines \
                 WHERE discipline_name = 'vengeance'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(origin, "sith_warrior");
        assert_eq!(combat_style, "juggernaut");

        // No utility / shared rows in disciplines.
        let leftovers: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM disciplines WHERE discipline_name IN ('utility', 'shared')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);

        // combat_style_shared_abilities: each ability fanned to BOTH styles.
        let leap_styles: Vec<String> = conn
            .prepare(
                "SELECT combat_style_codename FROM combat_style_shared_abilities \
                 WHERE ability_game_id = 'fl' ORDER BY combat_style_codename",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(leap_styles, vec!["juggernaut", "marauder"]);

        // discipline_abilities does NOT contain class_shared / utility / shared_mod rows.
        let post_rework_slots: Vec<String> = conn
            .prepare("SELECT DISTINCT slot_type FROM discipline_abilities")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(!post_rework_slots.iter().any(|s| s == "class_shared"));
        assert!(!post_rework_slots.iter().any(|s| s == "utility"));
        assert!(!post_rework_slots.iter().any(|s| s == "shared_mod"));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn populate_discipline_talents_routes_utility_to_class_utility_talents() {
        let path = temp_db_path("disc_tal_rework");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        seed_combat_styles_for(&db, "sith_warrior");

        // Combat-discipline talent (no fan-out).
        insert_obj(
            &db,
            "t1",
            "tal.sith_warrior.skill.vengeance.unyielding",
            "Talent",
        );
        // Per-origin utility talent: must fan to BOTH styles, NOT enter discipline_talents.
        insert_obj(
            &db,
            "t2",
            "tal.sith_warrior.skill.utility.interloper",
            "Talent",
        );

        // Disciplines must exist before discipline_talents -- run prerequisite.
        insert_obj(
            &db,
            "v1",
            "abl.sith_warrior.skill.vengeance.ravage",
            "Ability",
        );
        db.populate_disciplines().unwrap();

        let (disc_tal_count, util_count) = db.populate_discipline_talents().unwrap();
        assert_eq!(disc_tal_count, 1);
        assert_eq!(util_count, 2); // fanned to juggernaut + marauder

        let conn = db.conn.lock().unwrap();

        // discipline_talents: only the discipline-specific talent. No utility fan-out.
        let dt_rows: u64 = conn
            .query_row("SELECT COUNT(*) FROM discipline_talents", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dt_rows, 1);

        // class_utility_talents: utility talent fanned to both styles.
        let util_styles: Vec<String> = conn
            .prepare(
                "SELECT combat_style_codename FROM class_utility_talents \
                 WHERE talent_game_id = 't2' ORDER BY combat_style_codename",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(util_styles, vec!["juggernaut", "marauder"]);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// Every combat-style value emitted by `combat_style_for` must also appear
    /// in `origin_combat_styles(origin)`. The original FK gap shipped because
    /// the two maps drifted: `combat_style_for` returned 'sage'/'vanguard' while
    /// `origin_combat_styles` returned 'shadow'/'sage' and 'vanguard'/'commando'.
    /// This test catches that class of regression at `cargo test` time.
    #[test]
    fn discipline_map_combat_styles_match_origin_fan_out() {
        for (origin, discipline, combat_style) in DISCIPLINE_COMBAT_STYLE_MAP {
            let fan_out = origin_combat_styles(origin);
            assert!(
                fan_out.contains(combat_style),
                "({}, {}) -> {} not in origin_combat_styles({}) = {:?}",
                origin,
                discipline,
                combat_style,
                origin,
                fan_out,
            );
        }
    }

    /// Every origin in `PLAYER_ORIGINS` must yield exactly 2 combat styles --
    /// the two advanced classes that descend from that base class.
    #[test]
    fn every_player_origin_fans_to_two_combat_styles() {
        for origin in PLAYER_ORIGINS {
            let styles = origin_combat_styles(origin);
            assert_eq!(
                styles.len(),
                2,
                "origin {} should fan to 2 combat styles, got {:?}",
                origin,
                styles,
            );
        }
    }

    /// Each combat style emitted by either map must join cleanly to
    /// `combat_styles.fqn_segment` populated from `class.pc.advanced.<seg>`.
    /// The integration check uses a fixture that mirrors the live game's
    /// 16 advanced-class FQN segments.
    #[test]
    fn combat_style_values_join_combat_styles_fqn_segment() {
        // The 16 advanced-class internal codenames as they appear in
        // `class.pc.advanced.*` -- verified against live spice.sqlite.
        const ADVANCED_CLASS_FQN_SEGMENTS: &[&str] = &[
            "juggernaut",
            "marauder",
            "assassin",
            "sorcerer",
            "powertech",
            "mercenary",
            "operative",
            "sniper",
            "guardian",
            "sentinel",
            "shadow",
            "force_wizard",
            "specialist",
            "commando",
            "scoundrel",
            "gunslinger",
        ];

        for (origin, discipline, combat_style) in DISCIPLINE_COMBAT_STYLE_MAP {
            assert!(
                ADVANCED_CLASS_FQN_SEGMENTS.contains(combat_style),
                "DISCIPLINE_COMBAT_STYLE_MAP entry ({}, {}) -> '{}' is not a valid \
                 class.pc.advanced.<seg> codename. If SWTOR added/renamed an \
                 advanced class, update both this fixture AND the map.",
                origin,
                discipline,
                combat_style,
            );
        }

        for origin in PLAYER_ORIGINS {
            for combat_style in origin_combat_styles(origin) {
                assert!(
                    ADVANCED_CLASS_FQN_SEGMENTS.contains(combat_style),
                    "origin_combat_styles({}) yields '{}' which is not a valid \
                     class.pc.advanced.<seg> codename.",
                    origin,
                    combat_style,
                );
            }
        }
    }

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

    fn rec(prop_id: u16, value: f32) -> Vec<u8> {
        let mut v = Vec::with_capacity(6);
        v.extend_from_slice(&prop_id.to_le_bytes());
        v.extend_from_slice(&value.to_le_bytes());
        v
    }

    fn block(records: &[(u16, f32)]) -> Vec<u8> {
        // 4 bytes of pre-sentinel padding so the sentinel is not at byte 0
        // (matches the real payload shape where header bytes precede it).
        let mut v = vec![0xAAu8; 4];
        v.extend(rec(0x0401, -1.0));
        for (id, val) in records {
            v.extend(rec(*id, *val));
        }
        // Trailing non-0x04xx byte pair to terminate the block.
        v.extend_from_slice(&[0x80, 0x05, 0x00, 0x00, 0x00, 0x00]);
        v
    }

    #[test]
    fn ability_props_decode_cooldown() {
        let stats = scan_ability_props(&block(&[(0x0401, 15.0)]));
        assert_eq!(stats.cooldown, Some(15.0));
    }

    #[test]
    fn ability_props_decode_cast_and_force_cost() {
        let stats = scan_ability_props(&block(&[(0x041b, 1.5), (0x0403, 40.0)]));
        assert_eq!(stats.cast_time, Some(1.5));
        assert_eq!(stats.force_cost, Some(40));
    }

    #[test]
    fn ability_props_drop_low_force_cost_as_scaling_coefficient() {
        // 0x0403 = 1.0 on tech abilities is a coefficient, not a Force cost.
        let stats = scan_ability_props(&block(&[(0x0403, 1.0)]));
        assert_eq!(stats.force_cost, None);
    }

    #[test]
    fn ability_props_decode_resource_cost() {
        let stats = scan_ability_props(&block(&[(0x041e, 15.0)]));
        assert_eq!(stats.resource_cost, Some(15));
    }

    #[test]
    fn ability_props_no_sentinel_no_hits() {
        // Secondary template: no sentinel anywhere.
        let payload = vec![0x00; 64];
        let stats = scan_ability_props(&payload);
        assert!(!stats.any_hit());
    }

    #[test]
    fn ability_props_block_terminates_at_non_04xx() {
        // Sentinel + cooldown + non-0x04xx terminator + would-be-cast-time
        // outside the block. cast_time must NOT be picked up.
        let mut buf = vec![0xAAu8; 4];
        buf.extend(rec(0x0401, -1.0));
        buf.extend(rec(0x0401, 15.0));
        buf.extend_from_slice(&[0x80, 0x05, 0x00, 0x00, 0x00, 0x00]); // terminator
        buf.extend(rec(0x041b, 1.5)); // outside the block
        let stats = scan_ability_props(&buf);
        assert_eq!(stats.cooldown, Some(15.0));
        assert_eq!(stats.cast_time, None);
    }

    #[test]
    fn ability_props_sentinel_does_not_displace_real_cooldown() {
        // 0x0401 = -1.0 is the sentinel; the next 0x0401 carries cd.
        let stats = scan_ability_props(&block(&[(0x0401, 15.0)]));
        assert_eq!(stats.cooldown, Some(15.0));
    }

    #[test]
    fn resource_pool_warrior_is_rage() {
        assert_eq!(
            resource_pool_from_fqn("abl.sith_warrior.force_charge"),
            Some("rage")
        );
    }

    #[test]
    fn resource_pool_inquisitor_is_force() {
        assert_eq!(
            resource_pool_from_fqn("abl.sith_inquisitor.crushing_darkness"),
            Some("force")
        );
    }

    #[test]
    fn resource_pool_bounty_hunter_is_heat() {
        assert_eq!(
            resource_pool_from_fqn("abl.bounty_hunter.rocket_punch"),
            Some("heat")
        );
    }

    #[test]
    fn resource_pool_companion_is_none() {
        assert_eq!(
            resource_pool_from_fqn("abl.companion.weapon_set.blaster.tank.taunt"),
            None
        );
    }

    #[test]
    fn resource_pool_spvp_is_gsf() {
        assert_eq!(
            resource_pool_from_fqn("abl.spvp.missile.rocket_pod.damage"),
            Some("gsf")
        );
    }

    #[test]
    fn resource_pool_works_for_tal_prefix() {
        assert_eq!(
            resource_pool_from_fqn("tal.bounty_hunter.skill.bodyguard.empowered_scans"),
            Some("heat")
        );
        assert_eq!(
            resource_pool_from_fqn("tal.spvp.engine.barrel_roll.tier2"),
            Some("gsf")
        );
    }

    #[test]
    fn resource_pool_unknown_prefix_is_none() {
        assert_eq!(resource_pool_from_fqn("itm.something"), None);
        assert_eq!(resource_pool_from_fqn("qst.foo"), None);
    }

    fn synth_talent_with_tail(tail: &str) -> Vec<u8> {
        // Real tail format: filler bytes, then [len_u8][ascii bytes].
        let mut v = vec![0xCCu8; 64];
        v.push(tail.len() as u8);
        v.extend_from_slice(tail.as_bytes());
        v
    }

    #[test]
    fn talent_script_hook_extracts_tail_identifier() {
        let payload = synth_talent_with_tail("abl_bh_me_kolto_shot");
        assert_eq!(
            extract_talent_script_hook(&payload),
            Some("abl_bh_me_kolto_shot".to_string())
        );
    }

    #[test]
    fn talent_script_hook_extracts_long_gsf_hook() {
        let payload = synth_talent_with_tail("spvp_increasedsystemsdamagechance");
        assert_eq!(
            extract_talent_script_hook(&payload),
            Some("spvp_increasedsystemsdamagechance".to_string())
        );
    }

    #[test]
    fn talent_script_hook_returns_none_when_no_tail() {
        let payload = vec![0xFFu8; 32];
        assert_eq!(extract_talent_script_hook(&payload), None);
    }

    #[test]
    fn talent_script_hook_rejects_non_identifier_tail() {
        // Length-prefixed but contains a space — not a valid identifier.
        let mut payload = vec![0u8; 16];
        payload.push(11);
        payload.extend_from_slice(b"hello world");
        assert_eq!(extract_talent_script_hook(&payload), None);
    }
}
