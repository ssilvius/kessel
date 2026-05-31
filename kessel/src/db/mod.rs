//! SQLite database output with batched inserts

// Re-exported pub(crate) so the per-domain submodules can `use super::*` and
// get the whole shared surface (Result, params!, schema types, util helpers)
// without each file re-listing imports.
pub(crate) use anyhow::{Context, Result};
pub(crate) use rusqlite::{params, Connection, Transaction};
pub(crate) use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) use crate::grammar::Grammar;
pub(crate) use crate::schema::{decode_payload_schema_aware, GameObject};
pub(crate) use crate::stb::StbEntry;
mod ability;
mod conquest;
mod item;
mod npc;
pub mod passes;
mod quest;
mod schema;
mod schematic;
mod stats_systems;
mod world;
pub(crate) use world::find_planet_code_after_cc;
#[cfg(test)]
mod testutil;
mod util;
pub(crate) use util::*;

/// Quest class type_hi32 from client.gom (decoded by Agent D, legion 019e4d75).
const QUEST_CLASS_TYPE_HI32: u32 = 0x2ADE_C3D2;

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
    pub(crate) conn: Mutex<Connection>,
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
        // All DDL runs in one explicit transaction (atomic create). A direct
        // transaction -- not with_tx -- because with_tx flushes pending
        // objects/strings into tables that do not exist yet at schema time.
        // The two migrations run AFTER, on a fresh lock, preserving ordering.
        {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction()?;
            schema::create_core_tables(&tx)?;
            stats_systems::create_tables(&tx)?;
            schematic::create_tables(&tx)?;
            conquest::create_tables(&tx)?;
            world::create_tables(&tx)?;
            npc::create_tables(&tx)?;
            ability::create_tables(&tx)?;
            item::create_tables(&tx)?;
            quest::create_tables(&tx)?;
            tx.commit()?;
        }
        self.migrate_quest_typed_columns()?;
        self.migrate_disciplines_from_dis_columns()?;

        Ok(())
    }

    /// Insert one PBUK singleton prototype (#171). Singletons are zero-dot
    /// PBUK objects (master tables / config blobs) like tagTablePrototype,
    /// colCollectionItemsPrototype, cnqConquestInfoPrototype. They sit
    /// outside the `objects` table because their shape doesn't match the
    /// per-instance GameObject model (no per-instance GUID semantics, no
    /// kind label, no string_id linkage). The raw payload + a few cheap
    /// shape hints land in `singletons` for per-singleton decoders to
    /// consume in follow-on issues.
    pub fn insert_singleton(&self, obj: &crate::pbuk::GomObject) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let strings = obj.extract_strings();
        let string_count = strings.len() as i64;
        let cf_e0_count = count_byte_pattern(&obj.payload, &[0xCF, 0xE0, 0x00]) as i64;
        let cf_40_count = count_byte_pattern(&obj.payload, &[0xCF, 0x40, 0x00, 0x00]) as i64;
        let payload_b64 = BASE64.encode(&obj.payload);
        let header_hex = hex::encode(&obj.header);

        // extracted_at uses the table's DEFAULT (unixepoch()) by omitting the column.
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO singletons \
               (fqn, payload_size, payload_b64, string_count, cf_e0_count, cf_40_count, header_hex) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                obj.fqn,
                obj.payload.len() as i64,
                payload_b64,
                string_count,
                cf_e0_count,
                cf_40_count,
                header_hex,
            ],
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

    /// Run `f` inside a flushed write transaction: flush pending objects and
    /// strings, take the connection lock, open a transaction, run `f`, and
    /// commit on success (rolling back if `f` errors). This is the shared
    /// skeleton every `populate_*` method wraps -- the body closure does only
    /// the table-specific work, so the lock / transaction / commit ceremony
    /// lives in exactly one place.
    pub(crate) fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        self.flush()?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
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
}

impl Database {
    /// Load and base64-decode a singleton payload by FQN. Returns None when
    /// the singleton is absent or its base64 fails to decode. Shared by the
    /// per-singleton prototype decoders.
    fn load_singleton_payload(&self, fqn: &str) -> Option<Vec<u8>> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let conn = self.conn.lock().unwrap();
        let b64: Option<String> = conn
            .query_row(
                "SELECT payload_b64 FROM singletons WHERE fqn = ?1",
                [fqn],
                |r| r.get(0),
            )
            .ok();
        drop(conn);
        b64.and_then(|b| BASE64.decode(b).ok())
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
        // Quest reward links only (the summary groups this under "Quests").
        // mission_rewards is the superset; filter to qst.* for the same count
        // the dropped quest_rewards table reported.
        let reward_links: u64 = conn.query_row(
            "SELECT COUNT(*) FROM mission_rewards WHERE mission_fqn LIKE 'qst.%'",
            [],
            |row| row.get(0),
        )?;
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
pub(crate) const DISCIPLINE_COMBAT_STYLE_MAP: &[(&str, &str, &str)] = &[
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

const ABILITY_PROP_SENTINEL: [u8; 6] = [0x01, 0x04, 0x00, 0x00, 0x80, 0xBF];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

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
    fn init_schema_is_idempotent_and_creates_all_tables() {
        let path = temp_db_path("schema_idem");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        // Second run must not error (every CREATE is IF NOT EXISTS).
        db.init_schema().unwrap();
        let conn = db.conn.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n >= 71, "expected >=71 tables, got {n}");
    }

    #[test]
    fn typed_detail_tables_exist_after_init() {
        let path = temp_db_path("typed_details");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let names: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for t in ["npc_details", "schematic_details", "talent_details"] {
            assert!(names.contains(t), "missing typed-details table: {t}");
        }
    }

    #[test]
    fn cf40_marker_positions_finds_all_markers() {
        let mut p = vec![0x00, 0x00];
        p.extend_from_slice(&[0xCF, 0x40, 0x00, 0x00, 0xAA]); // marker at 2
        p.extend_from_slice(&[0x11, 0x22]);
        p.extend_from_slice(&[0xCF, 0x40, 0x00, 0x00, 0xBB]); // marker at 9
        assert_eq!(cf40_marker_positions(&p), vec![2, 9]);
    }

    #[test]
    fn find_length_prefixed_string_reads_after_prefix() {
        // ... 03 06 06 "medium" ...
        let mut r = vec![0x01, 0x02];
        r.extend_from_slice(&[0x03, 0x06, 0x06]);
        r.extend_from_slice(b"medium");
        r.push(0xCF);
        assert_eq!(
            find_length_prefixed_string(&r, &[0x03, 0x06]),
            Some("medium".to_string())
        );
    }

    #[test]
    fn find_length_prefixed_string_none_when_length_overruns() {
        let r = vec![0x03, 0x06, 0x40, b'x', b'y']; // len 0x40 but only 2 bytes follow
        assert_eq!(find_length_prefixed_string(&r, &[0x03, 0x06]), None);
        assert_eq!(find_length_prefixed_string(&r, &[0x09, 0x09]), None);
    }

    #[test]
    fn find_length_prefixed_string_none_when_record_shorter_than_prefix() {
        // Must not panic when the record is shorter than the prefix.
        assert_eq!(find_length_prefixed_string(&[0x03], &[0x03, 0x06]), None);
        assert_eq!(find_length_prefixed_string(&[], &[0x03, 0x06]), None);
    }

    #[test]
    fn typed_floats_in_extracts_and_skips_consumed_bytes() {
        // 04 <1.0> 01  04 <2.0> 0a
        let mut b = vec![0x04];
        b.extend_from_slice(&1.0f32.to_le_bytes());
        b.push(0x01);
        b.push(0x04);
        b.extend_from_slice(&2.0f32.to_le_bytes());
        b.push(0x0a);
        assert_eq!(typed_floats_in(&b), vec![1.0, 2.0]);
    }

    #[test]
    fn populate_gsf_crew_pairs_icon_with_animation() {
        let path = temp_db_path("gsf_crew");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Strings separated by non-printable bytes. risha is followed by an
        // animation; treek is followed by another icon (no animation); zenith
        // is the last string (no animation).
        let mut payload = Vec::new();
        for s in [
            "spvp_Crew_icon_risha",
            "dl_ponder_07",
            "spvp_Crew_icon_treek",
            "spvp_Crew_icon_zenith",
        ] {
            payload.push(0x00);
            payload.extend_from_slice(s.as_bytes());
        }
        payload.push(0x00);

        seed_singleton(&db, "scffCrewPrototype", &payload);
        let n = db.populate_gsf_crew().unwrap();
        assert_eq!(n, 3);

        let conn = db.conn.lock().unwrap();
        let row = |ord: i64| -> (String, String, Option<String>) {
            conn.query_row(
                "SELECT icon_name, crew_name, idle_animation FROM gsf_crew WHERE ordinal = ?1",
                params![ord],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(
            row(0),
            (
                "spvp_Crew_icon_risha".to_string(),
                "risha".to_string(),
                Some("dl_ponder_07".to_string())
            )
        );
        // treek's next string is another icon -> no animation.
        assert_eq!(
            row(1),
            (
                "spvp_Crew_icon_treek".to_string(),
                "treek".to_string(),
                None
            )
        );
        // zenith is last -> no animation.
        assert_eq!(
            row(2),
            (
                "spvp_Crew_icon_zenith".to_string(),
                "zenith".to_string(),
                None
            )
        );
    }
}
