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

            -- Quest objectives (#130, closes #15).
            -- Populated from the Quest class_ref array referenced by client.gom
            -- Quest property [38] (QuestObjective struct). Foundation pass
            -- records detected-objective count via marker scan; per-objective
            -- field decode (target_fqn, kind enum, count, name_string_id) lands
            -- in a follow-on PR once class_ref element byte-layout is verified.
            CREATE TABLE IF NOT EXISTS quest_objectives (
                quest_game_id   TEXT NOT NULL,
                quest_fqn       TEXT NOT NULL,
                ordinal         INTEGER NOT NULL,
                target_fqn      TEXT,
                kind            TEXT NOT NULL,
                count           INTEGER,
                name_string_id  INTEGER,
                raw_props       TEXT,
                PRIMARY KEY (quest_game_id, ordinal)
            );

            CREATE INDEX IF NOT EXISTS idx_quest_objectives_target ON quest_objectives(target_fqn);
            CREATE INDEX IF NOT EXISTS idx_quest_objectives_kind ON quest_objectives(kind);
            CREATE INDEX IF NOT EXISTS idx_quest_objectives_quest_fqn ON quest_objectives(quest_fqn);

            -- Per-step flag map (#212). Each quest payload encodes ALL its
            -- internal tracking flags inline as repeated CF40 markers at
            -- property hash 2ADEC3C7 (the same hash whose first occurrence
            -- populates quest_details.primary_tracking_flag). One row here
            -- per flag occurrence, ordered by byte position in the payload.
            --
            -- Drives kessel-warden's per-step quest matchers: combat-log
            -- events like RemoveEffect:InConversation, kill events, looting
            -- events fire named flags that match these strings. Categorize
            -- by prefix: jrn (journal display), track (progress tracker),
            -- hook (script trigger), qm (quest manager state), counter
            -- (kill-count), hyd (hydra event), branch_step (_bN_sN_tN game
            -- coordinate), quest_reward, spoke (action-specific), other.
            CREATE TABLE IF NOT EXISTS quest_objective_flags (
                quest_fqn       TEXT NOT NULL,
                ordinal         INTEGER NOT NULL,
                flag_name       TEXT NOT NULL,
                flag_category   TEXT NOT NULL,
                PRIMARY KEY (quest_fqn, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_qof_flag ON quest_objective_flags(flag_name);
            CREATE INDEX IF NOT EXISTS idx_qof_quest ON quest_objective_flags(quest_fqn);
            CREATE INDEX IF NOT EXISTS idx_qof_category ON quest_objective_flags(flag_category);

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

            -- GSF crew roster from the scffCrewPrototype singleton. One row
            -- per crew member: the icon resource name (spvp_Crew_icon_<name>),
            -- the bare crew_name (icon prefix stripped), and the idle
            -- animation reference that follows it. Self-validating via the
            -- companion names (risha, treek, zenith, dr_eckard_lokin, ...).
            CREATE TABLE IF NOT EXISTS gsf_crew (
                ordinal        INTEGER PRIMARY KEY,
                icon_name      TEXT NOT NULL,
                crew_name      TEXT NOT NULL,
                idle_animation TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_gsf_crew_name ON gsf_crew(crew_name);

            -- Companion roster from npc.companion.* objects. The canonical
            -- list of companions with display names resolved from the strings
            -- table (id2 = string_id, id1 = 0, en-us). category is the FQN
            -- segment after `npc.companion.` -- a class name (smuggler,
            -- jedi_knight, sith_warrior, bounty_hunter, spy, sith_sorcerer,
            -- ...) for origin-class companions, or a content source (alliance,
            -- mtx, kotet, kotfe, galactic_seasons, ...) otherwise.
            -- Informational source for downstream guides; supersedes the
            -- scattered companion references in other tables.
            CREATE TABLE IF NOT EXISTS companions (
                fqn           TEXT PRIMARY KEY,
                companion_key TEXT NOT NULL,
                name          TEXT,
                category      TEXT NOT NULL,
                string_id     INTEGER,
                guid          TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_companions_category ON companions(category);
            CREATE INDEX IF NOT EXISTS idx_companions_name ON companions(name);

            -- Item itemization tables. SWTOR item stats are computed, not
            -- stored per item: an item carries only (base level, quality,
            -- modifier set id), and these three tables turn that into numbers.
            -- Decoded from the itmRatingTablePrototype /
            -- itmBudgetedAttributesPrototype / itmModifierPackageTablePrototype
            -- singletons via the typed-value GOM grammar (gom_reader).
            --
            --   rating = item_rating_table[item_level][quality]
            --   stat   = item_budget_table[quality][item_level][permille]
            -- where the item's modifier package (item_modifier_packages, keyed
            -- by itmModifierSetID) supplies the per-stat permille index into
            -- the budget curve. Oracle: budget[artifact][89] holds 484 and 167.
            CREATE TABLE IF NOT EXISTS item_rating_table (
                item_level INTEGER NOT NULL,
                quality    TEXT NOT NULL,
                rating     INTEGER NOT NULL,
                PRIMARY KEY (item_level, quality)
            );

            -- Per-quality, per-level budget curve. permille is the 0..999 slot
            -- index (0.1% steps); a modifier package picks which permille feeds
            -- each stat. ~796k rows (4 qualities x ~199 levels x 1000 slots).
            CREATE TABLE IF NOT EXISTS item_budget_table (
                quality    TEXT NOT NULL,
                item_level INTEGER NOT NULL,
                permille   INTEGER NOT NULL,
                value      INTEGER NOT NULL,
                PRIMARY KEY (quality, item_level, permille)
            );

            -- Modifier package stat split. One row per (mod_id, stat): the
            -- permille of the slot budget that stat receives (single-stat mods
            -- = 1000; a 70/30 DPS armoring = strength 700 + endurance 300).
            CREATE TABLE IF NOT EXISTS item_modifier_packages (
                mod_id     INTEGER NOT NULL,
                stat_index INTEGER NOT NULL,
                stat_name  TEXT NOT NULL,
                permille   INTEGER NOT NULL,
                PRIMARY KEY (mod_id, stat_index)
            );

            -- The ability/proc an item grants when equipped. Decoded from the
            -- item payload's granted-ability field (GOM field id low32
            -- 0x2d7b8786, a UInt64 object guid). This is the "what does this
            -- item DO" link: legendary implants -> their bonus ability (e.g.
            -- Fearless Victor), set pieces -> set-bonus abilities, relics ->
            -- their proc.
            --
            -- ability_fqn / effect_text are resolved when the granted ability
            -- is itself an extracted object (true for legendary implant and
            -- tactical abilities, abl.itm.*). Relic procs reference UNNAMED
            -- effect objects that the extraction whitelist drops, so their
            -- ability_guid is recorded but ability_fqn/effect_text are NULL --
            -- surfacing exactly which procs still need the effect-object
            -- extraction (tracked follow-up).
            CREATE TABLE IF NOT EXISTS item_granted_abilities (
                item_fqn     TEXT PRIMARY KEY,
                item_game_id TEXT,
                ability_guid TEXT NOT NULL,
                ability_fqn  TEXT,
                ability_kind TEXT,
                effect_text  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_item_granted_abilities_guid
                ON item_granted_abilities(ability_guid);

            -- Per-item stat block: the actual stats an item provides, ready for
            -- a tooltip. Stats are FIXED on equippable gear and mod pieces --
            -- stored in the item payload's itmEquipModStats field (GOM field id
            -- low32 0xa4faffdd, a Map<STAT-enum, value>).
            --
            -- Each row is one (item, stat). The item metadata (level, quality,
            -- rating) is denormalized onto every row so a tooltip is a single
            -- `WHERE item_fqn=?` with no join. Moddable shells carry no innate
            -- stats (their stats come from slotted mods, which are themselves
            -- items with their own item_stats) and so produce no rows here --
            -- that is correct, not a gap. Item stats are fixed, not derived
            -- from a per-item modifier set, so the budget/modifier-package
            -- tables are for theorycrafting, not the per-item display path.
            --   item_level = itmBaseLevel (0xc7c48e7c)
            --   quality    = itmBaseQuality (0xc7c48e7d -> itmQuality member)
            --   rating     = display item rating (0x191f29c8)
            CREATE TABLE IF NOT EXISTS item_stats (
                item_fqn     TEXT NOT NULL,
                item_game_id TEXT,
                item_level   INTEGER,
                quality      TEXT,
                rating       INTEGER,
                stat_index   INTEGER NOT NULL,
                stat_name    TEXT NOT NULL,
                value        INTEGER NOT NULL,
                PRIMARY KEY (item_fqn, stat_index)
            );
            CREATE INDEX IF NOT EXISTS idx_item_stats_rating ON item_stats(rating);

            -- Relic proc classification. One row per relic item: the trigger
            -- (passive 'proc' vs activated 'onuse') and the stat it affects,
            -- classified from the relic FQN, plus the granted-ability guid
            -- (the proc/onuse ability the relic references via field
            -- 0x2d7b8786). See proc_stat() for the full set of stat labels.
            --
            -- IMPORTANT static-data ceiling: the EXACT proc magnitude,
            -- duration, and internal cooldown are NOT in the .tor archive.
            -- Proc-ability objects are shared across rating tiers while the
            -- proc value scales with the relic's rating at runtime, so the
            -- number is computed live (the str.abl.* proc strings carry blank
            -- duration/ICD tokens). The relic's STATIC equipped stats ARE
            -- captured (in item_stats). So this table answers "what kind of
            -- relic is this" deterministically; the live proc burst value is
            -- client-side residual (cf. #111).
            CREATE TABLE IF NOT EXISTS relic_procs (
                relic_fqn         TEXT PRIMARY KEY,
                relic_game_id     TEXT,
                trigger_kind      TEXT,   -- 'proc' (passive) or 'onuse' (activated); NULL if neither
                proc_stat         TEXT,   -- see proc_stat(): power/critical/mastery/defense/healing/absorb/alacrity/damage, or NULL
                proc_ability_guid TEXT    -- granted-ability guid (item field 0x2d7b8786)
            );
            CREATE INDEX IF NOT EXISTS idx_relic_procs_stat ON relic_procs(proc_stat);

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

            -- Item set membership (#105 part 1).
            --
            -- Set membership is encoded in the FQN itself, not in any pkg.*
            -- mediator (the issue's original pkg.* hypothesis was disproved
            -- by Phase 1 investigation: pkg.* contains 6 profession-trainer
            -- packages, unrelated to gear sets). Pattern:
            --   itm.setbonus.<source>.<class_group>.<subclass>.<set_id>.<slot>
            -- Members sharing the leading segments through <set_id> form a set.
            --
            -- Set display name comes from the .armor_box (or similar lockbox)
            -- member, whose str.itm.0.<id> string is the set's in-game name
            -- ("Berserker's Armor Lockbox" -> the set is "Berserker's Armor").
            -- Tier-bonus text (the 2/4/6-piece descriptions) lives in
            -- str.abl.* namespace and requires a separate resolver pass; it's
            -- not in this table yet -- documented follow-up.
            CREATE TABLE IF NOT EXISTS item_sets (
                set_fqn         TEXT PRIMARY KEY,
                source          TEXT,
                class_group     TEXT,
                name_string_id  INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_item_sets_source ON item_sets(source);

            CREATE TABLE IF NOT EXISTS item_set_members (
                item_game_id    TEXT NOT NULL,
                set_fqn         TEXT NOT NULL,
                slot            TEXT NOT NULL,
                PRIMARY KEY (item_game_id, set_fqn),
                FOREIGN KEY (item_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (set_fqn) REFERENCES item_sets(set_fqn)
            );

            CREATE INDEX IF NOT EXISTS idx_item_set_members_set ON item_set_members(set_fqn);

            -- Hydra script references (#214). Each hyd.* payload is a runtime
            -- trigger script encoded as plain GOM with FQN refs and command
            -- names stored inline as length-prefixed ASCII strings. No
            -- bytecode decryption needed — the names are right there.
            --
            -- One row per (hyd_fqn, ref_fqn) edge. ref_kind discriminates:
            --   - 'counter'     : qst.*.counter_* flag (kill/gather counter)
            --   - 'tracking'    : qst.*.track_* flag (step tracker)
            --   - 'journal'     : qst.*.jrn_* flag (journal entry trigger)
            --   - 'qm_state'    : qst.*.qm_* flag (quest manager state)
            --   - 'cnv_flag'    : qst.*.cnv_* flag (conversation-set flag)
            --   - 'glb_flag'    : qst.*.glb_* flag (global story flag)
            --   - 'hook'        : qst.*.hook_* flag (hydra script trigger)
            --   - 'target_npc'  : npc.* / enc.* / spn.* / plc.* target ref
            --   - 'conversation': cnv.* dialog ref
            --   - 'ability'     : abl.* ability spawn ref
            --   - 'quest_self'  : qst.* root ref (no trailing flag)
            --   - 'other_flag'  : qst.* with unrecognized suffix family
            --
            -- For warden Kill/Gather matchers: join counter rows to
            -- target_npc rows by hyd_fqn to find "this hyd's kill watch
            -- increments this counter when this NPC dies".
            CREATE TABLE IF NOT EXISTS hydra_refs (
                hyd_fqn      TEXT NOT NULL,
                ordinal      INTEGER NOT NULL,
                ref_kind     TEXT NOT NULL,
                ref_fqn      TEXT NOT NULL,
                PRIMARY KEY (hyd_fqn, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_hydra_refs_kind ON hydra_refs(ref_kind);
            CREATE INDEX IF NOT EXISTS idx_hydra_refs_ref  ON hydra_refs(ref_fqn);
            CREATE INDEX IF NOT EXISTS idx_hydra_refs_hyd  ON hydra_refs(hyd_fqn);

            -- NPC typed columns (#139) -- 32 props, 5 named enums from
            -- client.gom Npc schema.
            CREATE TABLE IF NOT EXISTS npc_details (
                fqn               TEXT PRIMARY KEY,
                difficulty        TEXT,
                faction           TEXT,
                class_role        TEXT,
                ai_template       TEXT,
                level             INTEGER
            );

            -- Schematic typed columns (#140) -- 35 props from Schematic schema.
            CREATE TABLE IF NOT EXISTS schematic_details (
                fqn               TEXT PRIMARY KEY,
                profession        TEXT,
                tier              INTEGER,
                training_cost     INTEGER
            );

            -- Talent typed columns (#140) -- 7 props from Talent schema.
            CREATE TABLE IF NOT EXISTS talent_details (
                fqn               TEXT PRIMARY KEY,
                discipline_code   TEXT,
                tree_position     INTEGER,
                tier              INTEGER
            );

            -- GSF requisition costs (#115 / #172). Decoded from the
            -- scFFComponentsCostPrototype + scFFComponentUpgradesCostPrototype
            -- singletons. (target_guid, cost_kind, tier) is the natural key;
            -- a single component appears once with cost_kind = 'component_unlock'
            -- and tier = 0, then five times with cost_kind = 'tier_upgrade' and
            -- tier = 1..5. target_game_id resolves the GUID into kessel's
            -- objects table (NULL when the component's content GUID isn't in
            -- the extracted set).
            CREATE TABLE IF NOT EXISTS gsf_requisition_costs (
                target_guid       TEXT NOT NULL,
                cost_kind         TEXT NOT NULL CHECK (cost_kind IN ('component_unlock', 'tier_upgrade')),
                tier              INTEGER NOT NULL,
                cost              INTEGER NOT NULL,
                target_game_id    TEXT,
                target_fqn        TEXT,
                art_path          TEXT,
                component_kind    TEXT,
                PRIMARY KEY (target_guid, cost_kind, tier)
            );
            CREATE INDEX IF NOT EXISTS idx_gsf_req_costs_target ON gsf_requisition_costs(target_game_id);
            CREATE INDEX IF NOT EXISTS idx_gsf_req_costs_kind ON gsf_requisition_costs(component_kind);

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

            -- Ability/talent -> effect block linkage (#173). One row per
            -- indexed CF E0 sub-record in the parent's payload. The parent
            -- self-reference is NOT included; this table is the structural
            -- linkage to the parent's effect-block sub-records that carry
            -- per-block typed properties (Weapon Damage, Modify Meta Stat,
            -- Play Appearance, Call Effect).
            --
            -- block_game_id is NULL when the effect block's GUID doesn't
            -- resolve to an extracted object (versioned-only ability
            -- category, issue #179). The raw block_guid is preserved so the
            -- unresolved edge stays visible in spice instead of being
            -- silently dropped.
            CREATE TABLE IF NOT EXISTS ability_effect_blocks (
                parent_game_id    TEXT NOT NULL,
                block_index       INTEGER NOT NULL,
                block_guid        TEXT NOT NULL,
                block_game_id     TEXT,
                PRIMARY KEY (parent_game_id, block_index),
                FOREIGN KEY (parent_game_id) REFERENCES objects(game_id)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_effect_blocks_block
                ON ability_effect_blocks(block_game_id);
            CREATE INDEX IF NOT EXISTS idx_ability_effect_blocks_guid
                ON ability_effect_blocks(block_guid);

            -- Ability action records. Decoded from CF40 markers
            -- E251D1CC (effAction), E251D1CD (effCondition),
            -- E251D1D0 (effInitializer), E71E2F92 (effLogicOp) in
            -- ability payloads. Scans BOTH canonical and non-canonical
            -- variants per FQN -- the rich effect data lives on the
            -- longer non-canonical variant (same pattern as PR #174 tags).
            -- Per-action parameter decode (damage values, multipliers)
            -- is deferred to the E251D1CE/CF parameter-list grammar work.
            CREATE TABLE IF NOT EXISTS ability_effects (
                ability_fqn       TEXT NOT NULL,
                ordinal           INTEGER NOT NULL,
                kind              TEXT NOT NULL CHECK (kind IN ('action','condition','initializer','logic_op')),
                enum_index        INTEGER NOT NULL,
                enum_member       TEXT NOT NULL,
                PRIMARY KEY (ability_fqn, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_effects_kind ON ability_effects(kind);
            CREATE INDEX IF NOT EXISTS idx_ability_effects_member ON ability_effects(enum_member);

            -- Ability effect numeric parameters. Decoded from the
            -- E251D1CE and E251D1CF parameter-list CF40 markers in abl
            -- payloads. Each row is one (param_flag, float_value) pair
            -- found inside the parameter-list tail.
            --
            -- Format inside the param list: `04 01 01 <flag_u8> <f32_LE>`
            -- (a typed-value triplet introducing a single float). param_flag
            -- empirically clusters around 0x10 (~90% of triplets) and 0x20
            -- (~10%); the semantic meaning of each flag varies per action
            -- and is not yet enum-resolved.
            --
            -- Floats outside the `04 01 01 XX <f32>` pattern (other
            -- parameter shapes) are NOT yet decoded -- this captures the
            -- subset that uses the well-formed primitive pattern.
            CREATE TABLE IF NOT EXISTS ability_effect_params (
                ability_fqn       TEXT NOT NULL,
                ordinal           INTEGER NOT NULL,
                source            TEXT NOT NULL CHECK (source IN ('CE','CF')),
                param_flag        INTEGER NOT NULL,
                value_f32         REAL NOT NULL,
                PRIMARY KEY (ability_fqn, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_effect_params_flag
                ON ability_effect_params(param_flag);

            -- Object CC marker references. CC is the second-layer marker
            -- family per docs/probes/dis-payload-format.md: every CC byte
            -- is followed by a 4-byte property-name hash + a variable-
            -- length value. CC markers are 4-10x more common than CF40
            -- markers per object (Quest: 474 CC vs 83 CF40; NPC: 11 CC
            -- vs 3 CF40). They're the alternative storage layer for the
            -- per-object typed data the CF40 walker doesn't catch.
            --
            -- The 4-byte CC ID is stored LE in payloads; the MAPPINGS.md
            -- known IDs are BE strings, so they're byte-reversed here.
            -- Known names (per MAPPINGS.md + legion 019e4e2a):
            --   37AE6F6F = stringRef        (BE: 6F6FAE37)
            --   0B84E217 = abilityRef       (BE: 17E2840B)
            --   03DDAFE4 = ?                (BE: E4AFDD03)
            --   2D31CD0C = ?                (BE: 0CCD312D)
            --   19D74B9D = ?                (BE: 9D4BD719)
            --   19D74B96 = ?                (BE: 964BD719)
            -- The full CC-hash-to-name dictionary is in a proprietary
            -- Bioware namespace; spike #144 to crack it (6 known names
            -- so far; ~700+ distinct IDs observed corpus-wide).
            --
            -- value_bytes_hex captures up to 16 bytes from the CC marker's
            -- value tail (limited because per-ID length grammar is unknown);
            -- a future PR per-ID grammar will extract typed values cleanly.
            CREATE TABLE IF NOT EXISTS object_cc_refs (
                object_game_id     TEXT NOT NULL,
                ordinal            INTEGER NOT NULL,
                cc_id_hex          TEXT NOT NULL,
                cc_known_name      TEXT,
                value_bytes_hex    TEXT NOT NULL,
                PRIMARY KEY (object_game_id, ordinal),
                FOREIGN KEY (object_game_id) REFERENCES objects(game_id)
            );
            CREATE INDEX IF NOT EXISTS idx_object_cc_refs_id ON object_cc_refs(cc_id_hex);
            CREATE INDEX IF NOT EXISTS idx_object_cc_refs_name ON object_cc_refs(cc_known_name);

            -- effAction parameter values. Within each effAction's parameter
            -- array, parameters are encoded as `<effParam_idx_u8><f32_LE>`.
            -- This is the per-action numeric data: damage coefficients,
            -- modifier amounts, condition values.
            --
            -- Verified for Massacre (effAction_BallisticImpulse):
            --   effParam_StandardHealthPercentMin = 0.1543
            --   effParam_IgnoreDualWieldModifier  = 1.54
            -- The 1.54 is the coefficient parsely displays as ~1.47.
            --
            -- Detection heuristic: inside the bytes after an effAction
            -- marker (until the next CF40 marker), scan for the pattern
            -- `<5-byte segment header><param_id_u8><f32_LE>` repeating.
            -- The segment header has the form `14 ED 0D 1E 3E ...` (more
            -- complex param shapes are not yet decoded).
            CREATE TABLE IF NOT EXISTS ability_action_params (
                ability_fqn       TEXT NOT NULL,
                effect_ordinal    INTEGER NOT NULL,
                param_ordinal     INTEGER NOT NULL,
                effparam_index    INTEGER NOT NULL,
                effparam_name     TEXT,
                value_f32         REAL NOT NULL,
                PRIMARY KEY (ability_fqn, effect_ordinal, param_ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_action_params_name
                ON ability_action_params(effparam_name);

            -- Damage-action CC parameter values. Per-effAction_Damage
            -- record, captures the 6 core CC parameter IDs + 2 optional
            -- ones, each with a 1-byte int8 value (CC + 4-byte ID + i8).
            --
            -- Verified across 1,452 occurrences each for the 6 core IDs:
            --   0135C0E0, 017459AB, 39285472, 0BB0D06E, 0176E21B, 011A6E3E
            -- Plus optional 3C0EB23D (342 occ), 0B9BBBDA (318 occ).
            --
            -- CC ID names are unknown (separate Bioware hash namespace per
            -- spike #144). The 1-byte values likely encode damage_type,
            -- modifier_type, target flag, etc. -- semantic resolution
            -- requires the hash crack or consumer-side reverse-mapping.
            --
            -- NOTE: the float damage coefficient (Massacre 1.47 per
            -- parsely) is NOT in these CC fields. It lives in a different
            -- encoding layer not yet characterized.
            CREATE TABLE IF NOT EXISTS ability_damage_params (
                ability_fqn       TEXT NOT NULL,
                effect_ordinal    INTEGER NOT NULL,
                param_ordinal     INTEGER NOT NULL,
                cc_id_hex         TEXT NOT NULL,
                value_i8          INTEGER NOT NULL,
                PRIMARY KEY (ability_fqn, effect_ordinal, param_ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_damage_params_cc
                ON ability_damage_params(cc_id_hex);

            -- Talent stat effects (PR: wire-typed-value-decoder).
            -- Decoded from CF40 D954FB02 markers in talent payloads.
            -- Each row: which stat the talent modifies and by how much.
            -- Verified format: <05><stat_idx_u8><01><04><float32_LE>.
            -- stat_name resolves to a member of the STAT enum (517 members);
            -- magnitude is a multiplier (e.g. 0.30 = +30%) per modStatType
            -- semantics. modStatType is in the D954FB04 effect-block class
            -- but is not yet decoded per-row here (deferred to per-property
            -- element grammar work).
            CREATE TABLE IF NOT EXISTS talent_stat_effects (
                talent_fqn        TEXT NOT NULL,
                ordinal           INTEGER NOT NULL,
                stat_index        INTEGER NOT NULL,
                stat_name         TEXT NOT NULL,
                magnitude         REAL NOT NULL,
                PRIMARY KEY (talent_fqn, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_talent_stat_effects_stat
                ON talent_stat_effects(stat_name);

            -- Tag dictionary (#174). Decoded from the `tagTablePrototype`
            -- singleton. ~6750 entries, all in the `tag.abl.*` namespace.
            -- Two marker forms:
            --   CE  → 7-byte hash (44 legacy records)
            --   CF  → 8-byte hash (6706 records)
            -- The hash is what abilities/talents reference in their payloads;
            -- `tag_fqn` is the human-readable name.
            CREATE TABLE IF NOT EXISTS tags (
                tag_hash       TEXT PRIMARY KEY,
                tag_fqn        TEXT NOT NULL,
                hash_marker    TEXT NOT NULL CHECK (hash_marker IN ('CE', 'CF'))
            );
            CREATE INDEX IF NOT EXISTS idx_tags_fqn ON tags(tag_fqn);

            -- Ability ↔ tag edges (#174). One row per (ability FQN, tag FQN)
            -- pair where the tag's hash bytes appear in any payload variant
            -- of that ability (canonical OR non-canonical -- tags often live
            -- on the longer non-canonical variant that kessel deduplicates).
            -- Aggregated per FQN so users get a stable tag set regardless of
            -- which variant happens to be canonical.
            CREATE TABLE IF NOT EXISTS ability_tags (
                ability_fqn        TEXT NOT NULL,
                ability_game_id    TEXT,
                tag_hash           TEXT NOT NULL,
                tag_fqn            TEXT NOT NULL,
                PRIMARY KEY (ability_fqn, tag_hash),
                FOREIGN KEY (tag_hash) REFERENCES tags(tag_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_ability_tags_tag ON ability_tags(tag_hash);
            CREATE INDEX IF NOT EXISTS idx_ability_tags_game_id ON ability_tags(ability_game_id);

            -- Appearance specs (#183). One row per .epp file at
            -- /resources/gamedata/epp/.../<name>.epp. FQN is the dotted
            -- form of the path-relative key. appearance_actions and
            -- fx_spec_refs are JSON arrays decoded from the XML body;
            -- raw_xml preserves the full XML for downstream typed-field
            -- consumers.
            CREATE TABLE IF NOT EXISTS appearance_specs (
                fqn                 TEXT PRIMARY KEY,
                appearance_actions  TEXT,
                fx_spec_refs        TEXT,
                raw_xml             TEXT NOT NULL
            );

            -- FX specs (#183). One row per .fxspec file. node_classes is a
            -- JSON array of the class names listed in the <classes> block;
            -- raw_xml preserves the full XML for per-node-instance
            -- consumers.
            CREATE TABLE IF NOT EXISTS fx_specs (
                fqn                 TEXT PRIMARY KEY,
                node_classes_json   TEXT NOT NULL,
                raw_xml             TEXT NOT NULL
            );

            -- SCPT compiled-native script bodies (#182, closes #127's
            -- consumer gap). One row per .scpt file at
            -- /resources/systemgenerated/compilednative/<numeric_id>.
            -- decoded_body is the post-XOR-decrypt body bytes (typically
            -- x86-64 UI/SFX native code per kessel/src/scpt.rs docs).
            -- Per-script semantic interpretation is a downstream consumer's
            -- job; this table provides the raw decrypted bytes.
            CREATE TABLE IF NOT EXISTS scripts (
                script_id          INTEGER PRIMARY KEY,
                decoded_size       INTEGER NOT NULL,
                decoded_body_b64   TEXT NOT NULL,
                extracted_at       INTEGER NOT NULL DEFAULT (unixepoch())
            );

            -- Talent ↔ tag edges (#174). Same shape as ability_tags.
            CREATE TABLE IF NOT EXISTS talent_tags (
                talent_fqn         TEXT NOT NULL,
                talent_game_id     TEXT,
                tag_hash           TEXT NOT NULL,
                tag_fqn            TEXT NOT NULL,
                PRIMARY KEY (talent_fqn, tag_hash),
                FOREIGN KEY (tag_hash) REFERENCES tags(tag_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_talent_tags_tag ON talent_tags(tag_hash);
            CREATE INDEX IF NOT EXISTS idx_talent_tags_game_id ON talent_tags(talent_game_id);
        "#,
    )?;
    Ok(())
}
