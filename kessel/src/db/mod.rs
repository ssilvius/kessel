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
mod item;
mod npc;
mod quest;
mod schema;
#[cfg(test)]
mod testutil;
mod util;
pub(crate) use util::*;

/// Quest class type_hi32 from client.gom (decoded by Agent D, legion 019e4d75).
const QUEST_CLASS_TYPE_HI32: u32 = 0x2ADE_C3D2;

/// Convert a `.fxspec` resource path into the path-relative key used by
/// `<fxSpecString>` references in `.epp` files. Returns None when the
/// path doesn't contain a `/fxspec/` segment or a `.fxspec` suffix. Used
/// by `populate_fx_specs` (#183) to make `appearance_specs.fx_spec_refs`
/// joinable to `fx_specs.fqn`.
///
/// Example: `/resources/art/fx/fxspec/abilities/sith_warrior/sw_massacre_sword_glow.fxspec`
/// → `abilities/sith_warrior/sw_massacre_sword_glow`.
fn fxspec_fqn_from_path(path: &str) -> Option<String> {
    let after_marker = path.split_once("/fxspec/")?.1;
    let trimmed = after_marker.strip_suffix(".fxspec")?;
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Recognize a SWTOR crafting profession from any segment of an FQN.
/// Returns the lowercased profession name when found, None otherwise. Used
/// by `populate_schematic_details_typed` (#178).
fn profession_from_fqn(fqn: &str) -> Option<String> {
    const PROFESSIONS: &[&str] = &[
        "artifice",
        "armormech",
        "armstech",
        "biochem",
        "cybertech",
        "synthweaving",
    ];
    let lower = fqn.to_lowercase();
    for prof in PROFESSIONS {
        if lower.contains(prof) {
            return Some((*prof).to_string());
        }
    }
    None
}

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
            schema::create_tables(&tx)?;
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

    /// Insert one row per PROT-magic .node file into the `objects` table
    /// (#175 entity layer for cnv.*, #181 extended to non-cnv prototypes
    /// like creature.*, stg.*, etc.).
    ///
    /// NODE files at `/resources/systemgenerated/prototypes/<num>.node` use
    /// the PROT format documented in `kessel/src/node.rs`. This populator
    /// walks every .node file with a valid PROT header, builds a synthetic
    /// GOM header so the existing `GameObject` constructor reads the
    /// content GUID the same way it does for PBUK objects, and emits one
    /// row per file. The `kind` column is derived from the FQN prefix by
    /// `from_gom_with_overrides`.
    ///
    /// Returns the number of NODE objects inserted.
    pub fn populate_node_objects(
        &self,
        tor_dir: &std::path::Path,
        hashes: &crate::hash::HashDictionary,
    ) -> Result<u64> {
        use crate::myp::Archive;
        use crate::pbuk::GomObject;
        use crate::schema::GameObject;
        use std::collections::HashSet;

        let proto_hashes: HashSet<u64> = hashes
            .paths_matching("/resources/systemgenerated/prototypes/")
            .into_iter()
            .map(|(h, _)| h)
            .collect();

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut inserted = 0u64;
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
                if !proto_hashes.contains(&entry.filename_hash) {
                    continue;
                }
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if data.len() < 20 || &data[..4] != b"PROT" {
                    continue;
                }
                let fqn_start = 0x14;
                let mut fqn_end = fqn_start;
                while fqn_end < data.len() && fqn_end < fqn_start + 200 && data[fqn_end] != 0 {
                    fqn_end += 1;
                }
                let fqn = match std::str::from_utf8(&data[fqn_start..fqn_end]) {
                    // Accept any FQN that contains a dot (kessel's basic
                    // shape check). Empty or non-dotted FQNs are likely
                    // corrupt PROT headers.
                    Ok(s) if s.contains('.') => s.to_string(),
                    _ => continue,
                };
                let payload_start = fqn_end + 1;
                if data.len() <= payload_start {
                    continue;
                }
                let payload = data[payload_start..].to_vec();

                // Build a synthetic 42-byte GOM header so from_gom_with_overrides
                // can read the content GUID at bytes 0..8 the same way it does
                // for PBUK objects. Template GUID slot (bytes 16..24) is left
                // zero because cnv objects share one all-cnv template constant
                // that is not yet wired into kessel.
                let mut header = vec![0u8; 42];
                header[0..8].copy_from_slice(&data[8..16]);

                let gom = GomObject {
                    fqn,
                    header,
                    payload,
                };
                let obj = GameObject::from_gom_with_overrides(&gom, None);
                self.insert_object(&obj)?;
                inserted += 1;
            }
        }
        self.flush()?;
        Ok(inserted)
    }

    /// Populate `schematic_details` with FQN-derived profession (#178).
    ///
    /// Walks every `schem.*` canonical object, looks for a recognized
    /// crafting profession token anywhere in the FQN, and records it.
    /// `tier` and `training_cost` remain NULL pending the per-property
    /// byte-layout decode work (the int8/16/32/enum_ref/string decode
    /// gap documented in CLAUDE.md).
    pub fn populate_schematic_details_typed(&self) -> Result<u64> {
        self.flush()?;
        let conn = self.conn.lock().unwrap();
        let fqns: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT fqn FROM objects WHERE fqn LIKE 'schem.%' AND is_canonical = 1")?;
            let collected: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };
        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO schematic_details \
               (fqn, profession, tier, training_cost) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut written = 0u64;
        for fqn in &fqns {
            let profession = profession_from_fqn(fqn);
            insert.execute(params![fqn, profession, None::<i64>, None::<i64>])?;
            written += 1;
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
    }

    /// Populate `appearance_specs` from every `.epp` file in the archives
    /// (#183).
    ///
    /// Each row carries the FQN extracted from the XML root attribute,
    /// JSON-encoded lists of distinct AppearanceAction types and fxSpec
    /// refs found in the body, and the raw decoded XML. Per-file decode
    /// failures are skipped silently rather than aborting the walk.
    pub fn populate_appearance_specs(
        &self,
        tor_dir: &std::path::Path,
        hashes: &crate::hash::HashDictionary,
    ) -> Result<u64> {
        use crate::myp::Archive;
        use crate::schema::epp;
        use std::collections::HashSet;

        let epp_hashes: HashSet<u64> = hashes
            .paths_matching(".epp")
            .into_iter()
            .filter(|(_, p)| p.ends_with(".epp"))
            .map(|(h, _)| h)
            .collect();

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut written = 0u64;
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO appearance_specs \
               (fqn, appearance_actions, fx_spec_refs, raw_xml) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
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
                if !epp_hashes.contains(&entry.filename_hash) {
                    continue;
                }
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let spec = match epp::decode_epp(&data) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let actions = serde_json::to_string(&spec.appearance_actions)?;
                let refs = serde_json::to_string(&spec.fx_spec_refs)?;
                insert.execute(params![spec.fqn, actions, refs, spec.raw_xml])?;
                written += 1;
            }
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
    }

    /// Populate `fx_specs` from every `.fxspec` file in the archives (#183).
    ///
    /// FQN is derived from the resource path between `/fxspec/` and the
    /// trailing `.fxspec`, matching the path-relative keys used by
    /// `appearance_specs.fx_spec_refs`. node_classes is JSON-encoded.
    pub fn populate_fx_specs(
        &self,
        tor_dir: &std::path::Path,
        hashes: &crate::hash::HashDictionary,
    ) -> Result<u64> {
        use crate::myp::Archive;
        use crate::schema::fxspec;
        use std::collections::HashSet;

        let fx_hashes: HashSet<(u64, String)> = hashes
            .paths_matching(".fxspec")
            .into_iter()
            .filter(|(_, p)| p.ends_with(".fxspec"))
            .map(|(h, p)| (h, p.clone()))
            .collect();
        let by_hash: std::collections::HashMap<u64, String> = fx_hashes.iter().cloned().collect();

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut written = 0u64;
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO fx_specs (fqn, node_classes_json, raw_xml) \
             VALUES (?1, ?2, ?3)",
        )?;
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
                let Some(path) = by_hash.get(&entry.filename_hash) else {
                    continue;
                };
                let Some(fqn) = fxspec_fqn_from_path(path) else {
                    continue;
                };
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let spec = match fxspec::decode_fxspec(&data, fqn) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let classes = serde_json::to_string(&spec.node_classes)?;
                insert.execute(params![spec.fqn, classes, spec.raw_xml])?;
                written += 1;
            }
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
    }

    /// Populate `scripts` with decrypted SCPT bodies (#182).
    ///
    /// Walks every `/resources/systemgenerated/compilednative/<numeric_id>`
    /// file, runs `kessel::scpt::parse_and_decrypt`, and persists the body
    /// (base64-encoded) plus the numeric_id from the SCPT header.
    /// Per-script semantic interpretation (combat formulas, GSF physics,
    /// UI script logic) lives downstream of this row; this populator
    /// supplies the raw decoded bytes so consumers don't re-decrypt.
    pub fn populate_scripts(
        &self,
        tor_dir: &std::path::Path,
        hashes: &crate::hash::HashDictionary,
    ) -> Result<u64> {
        use crate::myp::Archive;
        use crate::scpt;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use std::collections::HashSet;

        let scpt_hashes: HashSet<u64> = hashes
            .paths_matching("/resources/systemgenerated/compilednative/")
            .into_iter()
            .map(|(h, _)| h)
            .collect();

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut written = 0u64;
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO scripts (script_id, decoded_size, decoded_body_b64) \
             VALUES (?1, ?2, ?3)",
        )?;

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
                if !scpt_hashes.contains(&entry.filename_hash) {
                    continue;
                }
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let (header, body) = match scpt::parse_and_decrypt(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let body_b64 = BASE64.encode(&body);
                insert.execute(params![header.numeric_id as i64, body.len(), body_b64])?;
                written += 1;
            }
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
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

/// Search a record's bytes for the CC <cc_id> marker, then extract the
/// trailing length-prefixed `_pla_<planet>` ASCII string. Used by the
/// conquest events populator.
fn find_planet_code_after_cc(record: &[u8], cc_id: &[u8; 4]) -> Option<String> {
    let mut i = 0;
    while i + 5 <= record.len() {
        if record[i] == 0xCC && record[i + 1..i + 5] == *cc_id {
            // Scan ahead for the `_pla_<planet>` ASCII run.
            let tail = &record[i + 5..];
            for j in 0..tail.len().saturating_sub(5) {
                if &tail[j..j + 5] == b"_pla_" {
                    let mut end = j + 5;
                    while end < tail.len() {
                        let b = tail[end];
                        if !(b.is_ascii_alphanumeric() || b == b'_') {
                            break;
                        }
                        end += 1;
                    }
                    return std::str::from_utf8(&tail[j..end]).ok().map(String::from);
                }
            }
            return None;
        }
        i += 1;
    }
    None
}

impl Database {
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

    /// Populate `conquest_events` from the `cnqConquestInfoPrototype` singleton.
    /// Each event is a CF40 8C7DAFE5 record: marker + length-prefixed ASCII
    /// event name + inner sub-properties + a CC 0BAC73FD entry whose tail is
    /// a length-prefixed `_pla_<planet>` string.
    ///
    /// 90 records total. record_size is bimodal: single-planet invasions
    /// cluster at 68-80 bytes; themed/special events (Yavin, Onderon, Iokath,
    /// Rishi, etc.) cluster at 7700-8400 bytes with many sub-bonuses. A
    /// ~7400-byte empty gap separates the clusters, so any threshold inside
    /// the gap classifies cleanly. Observed split: 74 invasion / 16 themed.
    ///
    /// The threshold sits in that gap (not just above invasion sizes) on
    /// purpose: the final record has no following marker, so its measured
    /// size runs to end-of-payload and absorbs the singleton's trailing CFE0
    /// reference array. A tight threshold (e.g. 200) would mis-tag that one
    /// inflated invasion record as themed; a gap-centered threshold cannot.
    ///
    /// Returns rows inserted.
    pub fn populate_conquest_events(&self) -> Result<u64> {
        self.flush()?;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        const EVENT_MARKER: [u8; 4] = [0x8C, 0x7D, 0xAF, 0xE5];
        const PLANET_CC: [u8; 4] = [0x0B, 0xAC, 0x73, 0xFD];
        // Centered in the ~7400-byte gap between the invasion cluster (<=80B)
        // and the themed cluster (>=7700B). Robust to last-record inflation.
        const THEMED_SIZE_THRESHOLD: usize = 1000;

        let payload: Option<Vec<u8>> = {
            let conn = self.conn.lock().unwrap();
            let row: Option<String> = conn
                .query_row(
                    "SELECT payload_b64 FROM singletons WHERE fqn = 'cnqConquestInfoPrototype'",
                    [],
                    |r| r.get(0),
                )
                .ok();
            row.and_then(|b64| BASE64.decode(b64).ok())
        };
        let Some(payload) = payload else {
            return Ok(0);
        };

        // Find every 8C7DAFE5 marker position.
        let mut positions: Vec<usize> = Vec::new();
        let mut i = 0;
        while i + 9 <= payload.len() {
            if payload[i] == 0xCF
                && payload[i + 1] == 0x40
                && payload[i + 2] == 0x00
                && payload[i + 3] == 0x00
                && payload[i + 5..i + 9] == EVENT_MARKER
            {
                positions.push(i);
                i += 9;
            } else {
                i += 1;
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO conquest_events \
                    (ordinal, event_name, planet_code, event_kind, record_size) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (ord, &pos) in positions.iter().enumerate() {
                let end = positions.get(ord + 1).copied().unwrap_or(payload.len());
                let record = &payload[pos..end];

                // Event name: the first printable ASCII run of >= 4 chars
                // after the 9-byte CF40 marker. The name is preceded by a
                // `06 <len>` string tag, but we take the first qualifying run
                // rather than trust the length prefix -- robust against the
                // tag bytes themselves being non-printable separators.
                let name = extract_ascii_strings(&record[9..], 4).into_iter().next();

                // Planet code: find CC 0BAC73FD then the trailing
                // length-prefixed `_pla_<planet>` ASCII string.
                let planet = find_planet_code_after_cc(record, &PLANET_CC);

                if let Some(name) = name {
                    let kind = if record.len() >= THEMED_SIZE_THRESHOLD {
                        "themed"
                    } else {
                        "invasion"
                    };
                    insert
                        .execute(params![ord as i64, name, planet, kind, record.len() as i64,])?;
                    inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `conquest_schedule` from the `cnqSchedulePrototype` singleton:
    /// the weekly conquest rotation. Each CF40 record (after the header)
    /// carries an event reference (`cf`/`c7` tag + 8-byte GUID) and a week
    /// ordinal encoded as a big-endian u16 immediately after the constant
    /// `cb 01 ee fe bd 02 c9` anchor.
    ///
    /// The event GUID is resolved to a conquest event by locating those bytes
    /// inside `cnqConquestInfoPrototype` and mapping the offset to the
    /// enclosing CF40 8C7DAFE5 event record -- the same enumeration as
    /// `populate_conquest_events`, so `event_ordinal` aligns with
    /// `conquest_events.ordinal`. Returns rows inserted.
    pub fn populate_conquest_schedule(&self) -> Result<u64> {
        self.flush()?;
        const EVENT_MARKER: [u8; 4] = [0x8C, 0x7D, 0xAF, 0xE5];
        // Constant preamble in each schedule record, followed by the BE u16 week.
        const WEEK_ANCHOR: [u8; 7] = [0xCB, 0x01, 0xEE, 0xFE, 0xBD, 0x02, 0xC9];

        let Some(info) = self.load_singleton_payload("cnqConquestInfoPrototype") else {
            return Ok(0);
        };
        let Some(sched) = self.load_singleton_payload("cnqSchedulePrototype") else {
            return Ok(0);
        };

        // Event record start offsets, in order (ordinal == index, matching
        // conquest_events). Each runs until the next start or end-of-payload.
        let mut ev_starts: Vec<usize> = Vec::new();
        {
            let mut i = 0;
            while i + 9 <= info.len() {
                if info[i] == 0xCF
                    && info[i + 1] == 0x40
                    && info[i + 2] == 0
                    && info[i + 3] == 0
                    && info[i + 5..i + 9] == EVENT_MARKER
                {
                    ev_starts.push(i);
                    i += 9;
                } else {
                    i += 1;
                }
            }
        }
        let event_name = |idx: usize| -> Option<String> {
            let start = ev_starts[idx];
            let end = ev_starts.get(idx + 1).copied().unwrap_or(info.len());
            extract_ascii_strings(&info[start + 9..end], 4)
                .into_iter()
                .next()
        };
        // Map an 8-byte event GUID to the enclosing event record's ordinal.
        let event_for_guid = |guid: &[u8]| -> Option<usize> {
            let pos = info.windows(8).position(|w| w == guid)?;
            match ev_starts.binary_search(&pos) {
                Ok(i) => Some(i),
                Err(0) => None,
                Err(i) => Some(i - 1),
            }
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO conquest_schedule \
                    (week_ordinal, event_guid, event_ordinal, event_name) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            let positions = cf40_marker_positions(&sched);
            for (idx, &pos) in positions.iter().enumerate() {
                let end = positions.get(idx + 1).copied().unwrap_or(sched.len());
                let record = &sched[pos..end];

                // Event ref: first `cf` (non-marker) or `c7` after the 8-byte
                // marker+hash, followed by an 8-byte GUID.
                let mut guid: Option<&[u8]> = None;
                let mut j = 8;
                while j + 9 <= record.len() {
                    let is_ref = (record[j] == 0xCF
                        && !(record[j + 1] == 0x40 && record[j + 2] == 0 && record[j + 3] == 0))
                        || record[j] == 0xC7;
                    if is_ref {
                        guid = Some(&record[j + 1..j + 9]);
                        break;
                    }
                    j += 1;
                }
                let Some(guid) = guid else { continue };

                // Week: BE u16 immediately after the constant anchor.
                let Some(a) = record
                    .windows(WEEK_ANCHOR.len())
                    .position(|w| w == WEEK_ANCHOR)
                else {
                    continue;
                };
                let wk_pos = a + WEEK_ANCHOR.len();
                if wk_pos + 2 > record.len() {
                    continue;
                }
                let week = ((record[wk_pos] as i64) << 8) | record[wk_pos + 1] as i64;

                let guid_hex: String = guid.iter().map(|b| format!("{b:02x}")).collect();
                let ordinal = event_for_guid(guid);
                let name = ordinal.and_then(event_name);
                insert.execute(params![week, guid_hex, ordinal.map(|o| o as i64), name])?;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

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

    /// Populate `armor_classes` from the `cbtArmorTablePrototype` singleton.
    /// Each CF40 record carries a `02 <code>` byte and a `03 06 <len> <name>`
    /// length-prefixed class name. Returns rows inserted.
    pub fn populate_armor_classes(&self) -> Result<u64> {
        self.flush()?;
        let Some(payload) = self.load_singleton_payload("cbtArmorTablePrototype") else {
            return Ok(0);
        };

        // CF40 record positions.
        let positions = cf40_marker_positions(&payload);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO armor_classes (ordinal, code, name) VALUES (?1, ?2, ?3)",
            )?;
            for (ord, &pos) in positions.iter().enumerate() {
                let end = positions.get(ord + 1).copied().unwrap_or(payload.len());
                let record = &payload[pos..end];

                // Name: the length-prefixed string after the `03 06` tag.
                let name = find_length_prefixed_string(record, &[0x03, 0x06]);
                // Code: the byte after the leading `02` tag (record[10]).
                let code = record.get(10).copied();

                if let (Some(name), Some(code)) = (name, code) {
                    insert.execute(params![ord as i64, code as i64, name])?;
                    inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `stat_curve_values` from the `cbtShieldPerLevel` singleton.
    /// It is a multi-record, float-dense table: each CF40 record carries one
    /// or more typed floats (`04` tag + f32 LE). Every float is recorded in
    /// payload order, grouped by the enclosing CF40 field-hash.
    ///
    /// These are LITERAL stored values with no level/stat semantics: the
    /// curve is 2D (an undecoded segment key x level), so this is an honest
    /// raw dump for downstream prose/chart rendering, separated by curve_hash.
    ///
    /// The other per-level singletons (cbtArmorPerLevel, cbtStandardRatingInfo,
    /// chrGearScorePrototype) are single-record nested structures whose typed
    /// stream is not cleanly float-only; they need dedicated grammar work and
    /// are intentionally deferred so this table stays noise-free.
    ///
    /// Returns rows inserted.
    pub fn populate_stat_curve_values(&self) -> Result<u64> {
        self.flush()?;

        const PROTOTYPE: &str = "cbtShieldPerLevel";
        let Some(payload) = self.load_singleton_payload(PROTOTYPE) else {
            return Ok(0);
        };

        let positions = cf40_marker_positions(&payload);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            // Idempotent on re-run: clear this prototype's rows first. The
            // table has a surrogate PK with no natural-key collision, so a
            // plain re-insert would otherwise accumulate duplicates when
            // kessel runs against an existing output DB.
            tx.execute(
                "DELETE FROM stat_curve_values WHERE prototype = ?1",
                params![PROTOTYPE],
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT INTO stat_curve_values (prototype, curve_hash, ordinal, value) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            // Running ordinal per curve_hash.
            let mut ordinals: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            for (idx, &pos) in positions.iter().enumerate() {
                let end = positions.get(idx + 1).copied().unwrap_or(payload.len());
                let record = &payload[pos..end];
                // Field-hash: the 5 bytes after the `cf 40 00 00` marker.
                let curve_hash = record
                    .get(4..9)
                    .map(|h| h.iter().map(|b| format!("{b:02x}")).collect::<String>());
                // Every `04 <f32>` typed float in the record body (after the
                // 9-byte marker+hash).
                for value in typed_floats_in(&record[9.min(record.len())..]) {
                    let key = curve_hash.clone().unwrap_or_default();
                    let ord = ordinals.entry(key).or_insert(0);
                    insert.execute(params![PROTOTYPE, curve_hash, *ord, value as f64])?;
                    *ord += 1;
                    inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
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
    fn find_planet_code_after_cc_reads_pla_suffix() {
        // CC marker + cc_id, then a `_pla_<planet>` run terminated by a
        // non-[alnum|underscore] byte.
        let cc = [0x0B, 0xAC, 0x73, 0xFD];
        let mut record = vec![0xCC, 0x0B, 0xAC, 0x73, 0xFD];
        record.extend_from_slice(b"\x05x_pla_alderaan\x00more");
        assert_eq!(
            find_planet_code_after_cc(&record, &cc),
            Some("_pla_alderaan".to_string())
        );
    }

    #[test]
    fn find_planet_code_after_cc_returns_none_without_marker() {
        let cc = [0x0B, 0xAC, 0x73, 0xFD];
        // CC byte present but the following four bytes are not the cc_id.
        let record = vec![0xCC, 0x00, 0x00, 0x00, 0x00, b'_', b'p', b'l', b'a', b'_'];
        assert_eq!(find_planet_code_after_cc(&record, &cc), None);
    }

    #[test]
    fn find_planet_code_after_cc_returns_none_when_no_pla_run() {
        let cc = [0x0B, 0xAC, 0x73, 0xFD];
        let mut record = vec![0xCC, 0x0B, 0xAC, 0x73, 0xFD];
        record.extend_from_slice(b"no planet here");
        assert_eq!(find_planet_code_after_cc(&record, &cc), None);
    }

    #[test]
    fn populate_conquest_events_classifies_and_resists_last_record_inflation() {
        let path = temp_db_path("conquest_events");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // One synthetic event record: 9-byte CF40 8C7DAFE5 marker, then a
        // `06 <len> <name>` string, then a CC 0BAC73FD planet block ending in
        // a length-prefixed `_pla_<planet>` run with a non-printable
        // terminator. Mirrors the real cnqConquestInfoPrototype layout.
        fn event_record(name: &str, planet: &str) -> Vec<u8> {
            let mut r = vec![0xCF, 0x40, 0x00, 0x00, 0x00, 0x8C, 0x7D, 0xAF, 0xE5];
            r.push(0x06);
            r.push(name.len() as u8);
            r.extend_from_slice(name.as_bytes());
            r.extend_from_slice(&[0xCC, 0x0B, 0xAC, 0x73, 0xFD]);
            let pla = format!("_pla_{planet}");
            r.push(0x06);
            r.push(pla.len() as u8);
            r.extend_from_slice(pla.as_bytes());
            r.push(0x00);
            r
        }

        let mut payload = Vec::new();
        // Record A: small single-planet invasion.
        payload.extend_from_slice(&event_record("Alpha", "alpha"));
        // Record B: themed -- padded past the 1000-byte gap threshold.
        let b_start = payload.len();
        payload.extend_from_slice(&event_record("Beta", "beta"));
        payload.resize(b_start + 1200, 0x00);
        // Record C: small invasion, but it is the LAST record and is trailed
        // by ~280 bytes mimicking the singleton's top-level CFE0 array. Its
        // measured size (~320) exceeds the old 200 threshold yet stays well
        // inside the bimodal gap -- so it must still classify as invasion.
        payload.extend_from_slice(&event_record("Gamma", "gamma"));
        payload.resize(payload.len() + 280, 0x00);

        {
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO singletons \
                    (fqn, payload_size, payload_b64, string_count, cf_e0_count, cf_40_count, header_hex) \
                 VALUES ('cnqConquestInfoPrototype', ?1, ?2, 0, 0, 3, '')",
                params![payload.len() as i64, BASE64.encode(&payload)],
            )
            .unwrap();
        }

        let inserted = db.populate_conquest_events().unwrap();
        assert_eq!(inserted, 3);

        let conn = db.conn.lock().unwrap();
        let row = |ord: i64| -> (String, String, Option<String>, i64) {
            conn.query_row(
                "SELECT event_name, event_kind, planet_code, record_size \
                 FROM conquest_events WHERE ordinal = ?1",
                params![ord],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
        };

        let a = row(0);
        assert_eq!(a.0, "Alpha");
        assert_eq!(a.1, "invasion");
        assert_eq!(a.2.as_deref(), Some("_pla_alpha"));

        let b = row(1);
        assert_eq!(b.0, "Beta");
        assert_eq!(b.1, "themed");

        // The crux: an inflated final invasion record must not become themed.
        let c = row(2);
        assert_eq!(c.0, "Gamma");
        assert_eq!(c.1, "invasion");
        assert_eq!(c.2.as_deref(), Some("_pla_gamma"));
        assert!(
            c.3 > 200,
            "last record size {} must exceed the old 200 threshold to guard the fix",
            c.3
        );
        assert!(
            c.3 < 1000,
            "last record size {} must stay within the gap",
            c.3
        );
    }

    #[test]
    fn populate_conquest_schedule_resolves_weeks_to_events() {
        let path = temp_db_path("conquest_schedule");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        let guid_a: [u8; 8] = [0x07, 0x1d, 0x4c, 0xff, 0x6c, 0x65, 0xff, 0xfe];
        let guid_b: [u8; 8] = [0x7c, 0x32, 0xfd, 0x18, 0x78, 0x30, 0x33, 0xab];
        let guid_x: [u8; 8] = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33];

        // Each conquest event record: CF40 8C7DAFE5 marker, 06 <len> <name>,
        // then its identifying GUID embedded in the body.
        fn event_record(name: &str, guid: &[u8; 8]) -> Vec<u8> {
            let mut r = vec![0xCF, 0x40, 0x00, 0x00, 0x43, 0x8C, 0x7D, 0xAF, 0xE5];
            r.push(0x06);
            r.push(name.len() as u8);
            r.extend_from_slice(name.as_bytes());
            r.extend_from_slice(&[0x01, 0x01]);
            r.extend_from_slice(guid);
            r
        }
        let mut info = Vec::new();
        info.extend_from_slice(&event_record("Alpha", &guid_a));
        info.extend_from_slice(&event_record("Beta", &guid_b));
        seed_singleton(&db, "cnqConquestInfoPrototype", &info);

        // Each schedule record: marker + hash, 03 02, cf <guid>, then the
        // week anchor `cb 01 ee fe bd 02 c9` + BE u16 week, then trailer.
        fn sched_record(guid: &[u8; 8], week: u16) -> Vec<u8> {
            let mut r = vec![0xCF, 0x40, 0x00, 0x00, 0x43, 0x8D, 0xCC, 0x77];
            r.extend_from_slice(&[0x03, 0x02, 0xCF]);
            r.extend_from_slice(guid);
            r.extend_from_slice(&[0xCB, 0x01, 0xEE, 0xFE, 0xBD, 0x02, 0xC9]);
            r.extend_from_slice(&week.to_be_bytes());
            r.extend_from_slice(&[0x02, 0x02]);
            r
        }
        let mut sched = Vec::new();
        sched.extend_from_slice(&sched_record(&guid_a, 1001)); // -> Alpha
        sched.extend_from_slice(&sched_record(&guid_b, 1002)); // -> Beta
        sched.extend_from_slice(&sched_record(&guid_x, 1003)); // unresolved GUID
        seed_singleton(&db, "cnqSchedulePrototype", &sched);

        let n = db.populate_conquest_schedule().unwrap();
        assert_eq!(n, 3);

        let conn = db.conn.lock().unwrap();
        let row = |wk: i64| -> (String, Option<i64>, Option<String>) {
            conn.query_row(
                "SELECT event_guid, event_ordinal, event_name FROM conquest_schedule \
                 WHERE week_ordinal = ?1",
                params![wk],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(
            row(1001),
            (
                "071d4cff6c65fffe".to_string(),
                Some(0),
                Some("Alpha".to_string())
            )
        );
        assert_eq!(
            row(1002),
            (
                "7c32fd18783033ab".to_string(),
                Some(1),
                Some("Beta".to_string())
            )
        );
        // Unresolved GUID: row exists, event_ordinal/name NULL.
        assert_eq!(row(1003), ("deadbeef00112233".to_string(), None, None));
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
    fn populate_armor_classes_decodes_code_and_name() {
        let path = temp_db_path("armor_classes");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Two records: marker(9) + 02 <code> 01 01 + 03 06 <len> <name>.
        fn armor_record(hash5: &[u8; 5], code: u8, name: &str) -> Vec<u8> {
            let mut r = vec![0xCF, 0x40, 0x00, 0x00];
            r.extend_from_slice(hash5);
            r.extend_from_slice(&[0x02, code, 0x01, 0x01]);
            r.extend_from_slice(&[0x03, 0x06, name.len() as u8]);
            r.extend_from_slice(name.as_bytes());
            r
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&armor_record(
            &[0x48, 0xa2, 0xf9, 0x99, 0xa3],
            0x5b,
            "medium",
        ));
        payload.extend_from_slice(&armor_record(
            &[0x48, 0xa2, 0xf9, 0x99, 0xa3],
            0x5a,
            "light",
        ));

        seed_singleton(&db, "cbtArmorTablePrototype", &payload);
        let n = db.populate_armor_classes().unwrap();
        assert_eq!(n, 2);

        let conn = db.conn.lock().unwrap();
        let (code, name): (i64, String) = conn
            .query_row(
                "SELECT code, name FROM armor_classes WHERE ordinal = 0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(code, 0x5b);
        assert_eq!(name, "medium");
        let light: String = conn
            .query_row(
                "SELECT name FROM armor_classes WHERE ordinal = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(light, "light");
    }

    #[test]
    fn populate_stat_curve_values_groups_floats_by_curve_hash() {
        let path = temp_db_path("stat_curves");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Two 17-byte shield records sharing one hash, then one with another.
        fn shield_record(hash5: &[u8; 5], value: f32, idx: u8) -> Vec<u8> {
            let mut r = vec![0xCF, 0x40, 0x00, 0x00];
            r.extend_from_slice(hash5);
            r.push(0x04);
            r.extend_from_slice(&value.to_le_bytes());
            r.extend_from_slice(&[idx, 0x0a, 0x01]);
            r
        }
        let h1 = [0x27, 0x9b, 0x23, 0xb1, 0x2e];
        let h2 = [0x27, 0x9b, 0x23, 0xb1, 0x2f];
        let mut payload = Vec::new();
        payload.extend_from_slice(&shield_record(&h1, 42.0, 0x02));
        payload.extend_from_slice(&shield_record(&h1, 45.0, 0x03));
        payload.extend_from_slice(&shield_record(&h2, 100.0, 0x02));

        seed_singleton(&db, "cbtShieldPerLevel", &payload);
        let n = db.populate_stat_curve_values().unwrap();
        assert_eq!(n, 3);

        // Idempotent on re-run: a second populate must not duplicate rows.
        db.populate_stat_curve_values().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let total: i64 = conn
                .query_row("SELECT COUNT(*) FROM stat_curve_values", [], |r| r.get(0))
                .unwrap();
            assert_eq!(total, 3, "re-run must not duplicate stat_curve_values rows");
        }

        let conn = db.conn.lock().unwrap();
        // First hash: ordinals 0,1 with values 42,45.
        let vals: Vec<(i64, f64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT ordinal, value FROM stat_curve_values \
                     WHERE curve_hash = '279b23b12e' ORDER BY ordinal",
                )
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(vals, vec![(0, 42.0), (1, 45.0)]);
        // Second hash restarts ordinal at 0.
        let (ord, val): (i64, f64) = conn
            .query_row(
                "SELECT ordinal, value FROM stat_curve_values WHERE curve_hash = '279b23b12f'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((ord, val), (0, 100.0));
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
