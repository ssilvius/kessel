//! Quest, mission, chain, and cluster extraction: populators, FQN
//! classifiers, table DDL, and tests for the quest domain.

use super::*;
use crate::quest;

impl Database {
    /// Add typed columns to `quest_details` from the client.gom Quest schema
    /// (#129 foundation). Idempotent -- checks pragma_table_info first so
    /// re-runs are no-ops.
    ///
    /// Foundation-only scope: columns store marker-presence flags ("PRESENT")
    /// rather than decoded enum members / ints / strings. Real value decode
    /// requires per-property post-CF40 byte-layout verification and ships in
    /// a follow-on PR.
    pub fn migrate_quest_typed_columns(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let existing: std::collections::HashSet<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(quest_details)")?;
            let cols = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            cols.into_iter().collect()
        };
        let additions = [
            ("activity_type", "TEXT"),
            ("difficulty", "TEXT"),
            ("rewards_visibility", "TEXT"),
            ("episode_season", "TEXT"),
            ("level", "INTEGER"),
            // Primary tracking flag string (hash 2ADEC3C7). One per quest;
            // string_id reference like "track_*", "jrn_*", "glb_*", "cnv_*",
            // "complex_*" identifying the quest's main progress flag.
            ("primary_tracking_flag", "TEXT"),
            // Raw int8 codes extracted by hi32 hash. The GOM schema labels
            // these as kind=int8 with no enum_ref linkage, but their value
            // distributions are enum-like (small distinct-value sets across
            // 1500+ quests). Issue #210. Distributions verified empirically
            // via quest_hash_stats discovery binary:
            //
            //   308A97A4: 17 distinct, top 1(1056), 3(184), 4(98), 6(38), 2(38)
            //   308A9B02: 26 distinct, top 2(517), 3(266), 4(139), 5(110)
            //   4C85BFC6:  9 distinct, top 1(1413), 4(28), 5(19), 3(15)
            //
            // Column names are honest-but-tentative; likely correspond to
            // qstActivityType / qstDifficulty / qstRewardsVisibility but
            // enum-name resolution is a separate investigation.
            ("mission_type_code", "INTEGER"),
            ("category_code", "INTEGER"),
            ("visibility_code", "INTEGER"),
        ];
        for (name, ty) in additions {
            if !existing.contains(name) {
                let sql = format!("ALTER TABLE quest_details ADD COLUMN {name} {ty}");
                conn.execute(&sql, [])?;
            }
        }
        Ok(())
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

    /// Quest enums. Real value decode lands in a follow-on PR.
    pub fn populate_quest_details_typed(&self) -> Result<u64> {
        self.flush()?;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, json FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
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
        let mut updated = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE quest_details
                    SET activity_type = ?1,
                        difficulty = ?2,
                        rewards_visibility = ?3,
                        episode_season = ?4,
                        level = ?5,
                        primary_tracking_flag = ?6,
                        mission_type_code = ?7,
                        category_code = ?8,
                        visibility_code = ?9
                  WHERE fqn = ?10",
            )?;
            for (fqn, json_str) in &rows {
                let payload = match serde_json::from_str::<serde_json::Value>(json_str)
                    .ok()
                    .and_then(|v| {
                        v.get("payload_b64")
                            .and_then(|p| p.as_str())
                            .map(String::from)
                    })
                    .and_then(|b64| BASE64.decode(b64).ok())
                {
                    Some(bytes) => bytes,
                    None => continue,
                };
                let decoded = match decode_payload_schema_aware(&payload, QUEST_CLASS_TYPE_HI32) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let named = decoded.named_props.as_object();
                // Pull the decoded value for each enum property. The walker
                // now emits real values per docs/probes/typed-value-encoding.md
                // (PRs #199-#201); enum properties decode to {"enum": "<name>",
                // "index": N, "name": "<member>"}.
                let enum_member = |needle: &str| -> Option<String> {
                    let m = named?;
                    let (_k, v) = m.iter().find(|(k, _)| k.contains(needle))?;
                    v.get("name").and_then(|n| n.as_str()).map(String::from)
                };
                let int_value = |needle: &str| -> Option<i64> {
                    let m = named?;
                    let (_k, v) = m.iter().find(|(k, _)| k.contains(needle))?;
                    v.as_i64()
                };
                // Extract a typed property by its hi32 hash suffix. The
                // walker mislabels the kind of some properties (5 of 10
                // kinds wrong in the GOM dictionary), so name lookup is
                // unreliable; suffix lookup on `__<HI32>` is canonical.
                let prop_by_hash = |hi32: &str| -> Option<&serde_json::Value> {
                    let m = named?;
                    let suffix = format!("__{hi32}");
                    m.iter().find(|(k, _)| k.ends_with(&suffix)).map(|(_, v)| v)
                };
                // 0xCE is a metadata-layer marker byte that the walker
                // sometimes misreads as a property int8 value. Filtered
                // out of raw-int extractions; legitimate enum values for
                // the three quest hashes used here are small positive
                // integers (1-26 range).
                const CE_MARKER_AS_INT8: i64 = -50;
                let int_for_hash = |hi32: &str| -> Option<i64> {
                    let n = prop_by_hash(hi32)?.as_i64()?;
                    if n == CE_MARKER_AS_INT8 {
                        None
                    } else {
                        Some(n)
                    }
                };
                let string_for_hash = |hi32: &str| -> Option<String> {
                    prop_by_hash(hi32)?.as_str().map(String::from)
                };
                // Enum properties are keyed by hash, not name, in the decoded
                // output -- so the name-based `enum_member` lookup never matched
                // (all four columns landed empty across every quest). Resolve by
                // the canonical `__<HI32>` suffix instead, the same path the
                // tracking-flag / *_code columns already use successfully (#269).
                // The walker either emits the member name directly, or a raw
                // enum index (it mislabels some enum kinds as int8) -- handle
                // both, resolving the index against the schema's member list.
                let enum_by_hash = |hi32: &str, enum_name: &str| -> Option<String> {
                    let v = prop_by_hash(hi32)?;
                    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                        return Some(name.to_string());
                    }
                    let idx = v.as_i64()?;
                    if idx < 0 {
                        return None;
                    }
                    crate::gom_schema::enum_for_name(enum_name)
                        .and_then(|e| e.members.get(idx as usize).cloned())
                };
                // Hashes are the first 8 hex of each property id and match the
                // Quest class property_refs (e.g. 4000004EB58CE5FC).
                let activity = enum_by_hash("B58CE5FC", "qstActivityType")
                    .or_else(|| enum_member("qstActivityType"));
                let difficulty = enum_by_hash("B0E9BAF2", "qstDifficulty")
                    .or_else(|| enum_member("qstDifficulty"));
                let rewards = enum_by_hash("1D4649A2", "qstRewardsVisibility")
                    .or_else(|| enum_member("qstRewardsVisibility"));
                let episode = enum_by_hash("D3680699", "qstEpisodeSeason")
                    .or_else(|| enum_member("qstEpisodeSeason"));
                // Level: per-property level field may be int8 stored as
                // an int value (not a wrapped struct). Try direct decode.
                let level = int_value("Level").or_else(|| int_value("level"));
                let tracking_flag = string_for_hash("2ADEC3C7");
                let mission_type_code = int_for_hash("308A97A4");
                let category_code = int_for_hash("308A9B02");
                let visibility_code = int_for_hash("4C85BFC6");
                let affected = stmt.execute(params![
                    activity,
                    difficulty,
                    rewards,
                    episode,
                    level,
                    tracking_flag,
                    mission_type_code,
                    category_code,
                    visibility_code,
                    fqn,
                ])?;
                if affected > 0 {
                    updated += 1;
                }
            }
        }
        tx.commit()?;
        Ok(updated)
    }

    /// Populate `quest_objective_flags` (#212) by walking every CF40 marker
    /// for property hash 2ADEC3C7 in each canonical Quest payload. Each
    /// occurrence carries a wrapped length-prefixed string identifying one
    /// of the quest's internal progression flags.
    ///
    /// The schema-aware walker collapses repeated property markers into a
    /// single `named_props` entry (last-write-wins); this populator scans
    /// the raw payload bytes directly to preserve every occurrence in
    /// byte-position order.
    ///
    /// Categorizes each flag by its leading underscore-delimited segment:
    /// `jrn`, `track`, `hook`, `qm`, `counter`, `hyd`, `quest_reward`,
    /// `spoke`, plus `branch_step` for the `_bN_sN_tN` form, else `other`.
    ///
    /// Returns total rows inserted.
    pub fn populate_quest_objective_flags(&self) -> Result<u64> {
        self.flush()?;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        const TARGET_HI32: [u8; 4] = [0x2A, 0xDE, 0xC3, 0xC7];

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            fetch_fqn_payloads(&conn, "Quest")?
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO quest_objective_flags \
                    (quest_fqn, ordinal, flag_name, flag_category) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (fqn, b64) in &rows {
                let Ok(payload) = BASE64.decode(b64) else {
                    continue;
                };
                let mut ordinal: i64 = 0;
                let mut i = 0;
                while i + 12 <= payload.len() {
                    let is_target = payload[i] == 0xCF
                        && payload[i + 1] == 0x40
                        && payload[i + 2] == 0x00
                        && payload[i + 3] == 0x00
                        && payload[i + 5..i + 9] == TARGET_HI32;
                    if !is_target {
                        i += 1;
                        continue;
                    }
                    // Value starts at i+9. Real payloads use a wrapper
                    // (0x01 or 0x07) followed by inner string tag (0x06)
                    // and length byte. Bare 0x06 also accepted for safety.
                    let value_offset = match payload[i + 9] {
                        0x01 | 0x07 if i + 12 < payload.len() && payload[i + 10] == 0x06 => {
                            Some(i + 11)
                        }
                        0x06 => Some(i + 10),
                        _ => None,
                    };
                    if let Some(len_off) = value_offset {
                        let ln = payload[len_off] as usize;
                        let str_start = len_off + 1;
                        let str_end = str_start + ln;
                        if ln > 0 && str_end <= payload.len() {
                            if let Ok(s) = std::str::from_utf8(&payload[str_start..str_end]) {
                                let cat = classify_quest_flag(s);
                                stmt.execute(params![fqn, ordinal, s, cat])?;
                                inserted += 1;
                                ordinal += 1;
                            }
                        }
                    }
                    i += 9;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `quest_milestones` (#265): isolate the per-quest completion
    /// signal -- the `qm_*`/`go_*` milestone declaration a quest sets when it
    /// finishes -- as one clean row, so kessel-warden has an unambiguous
    /// "this quest is done" target instead of grepping `quest_objective_flags`.
    ///
    /// Pure derivation over the already-populated `quest_objective_flags`
    /// (no payload re-decode): the source rows are every flag with
    /// `flag_category = 'qm'` plus every `go_*` flag (which classify as
    /// 'other'). `is_terminal = 1` marks the byte-order-last qm/go flag per
    /// quest -- `ordinal` already encodes byte position and is unique per
    /// quest, so exactly one milestone per quest is terminal.
    ///
    /// Cross-quest prerequisite matching is deliberately NOT done here: those
    /// `qm_*`/`has_*` variables are internal per-quest state, proven to yield
    /// zero cross-quest edges (vault prereq_finding.md). Returns rows written.
    pub fn populate_quest_milestones(&self) -> Result<u64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let written = tx.execute(
            r#"
            INSERT OR REPLACE INTO quest_milestones (quest_fqn, milestone_name, is_terminal)
            SELECT f.quest_fqn,
                   f.flag_name,
                   CASE WHEN MAX(f.ordinal) = (
                       SELECT MAX(g.ordinal) FROM quest_objective_flags g
                       WHERE g.quest_fqn = f.quest_fqn
                         AND (g.flag_category = 'qm' OR g.flag_name LIKE 'go\_%' ESCAPE '\')
                   ) THEN 1 ELSE 0 END
            FROM quest_objective_flags f
            WHERE f.flag_category = 'qm' OR f.flag_name LIKE 'go\_%' ESCAPE '\'
            GROUP BY f.quest_fqn, f.flag_name
            "#,
            [],
        )? as u64;
        tx.commit()?;
        Ok(written)
    }

    /// Populate `hydra_refs` (#214) by scanning hyd.* payloads for inline
    /// ASCII FQN references. Hydra scripts encode their references (counter
    /// flags, target NPCs, conversations, etc.) as length-prefixed ASCII
    /// strings inside the GOM payload — no bytecode decode required.
    ///
    /// Each extracted FQN is classified by prefix and suffix family:
    ///   - qst.*.counter_*    -> 'counter'  (kill/gather counter)
    ///   - qst.*.track_*      -> 'tracking'
    ///   - qst.*.jrn_*        -> 'journal'
    ///   - qst.*.qm_*         -> 'qm_state'
    ///   - qst.*.cnv_*        -> 'cnv_flag'
    ///   - qst.*.glb_*        -> 'glb_flag'
    ///   - qst.*.hook_*       -> 'hook'
    ///   - qst.* root         -> 'quest_self'
    ///   - qst.* other suffix -> 'other_flag'
    ///   - cnv.*              -> 'conversation'
    ///   - abl.*              -> 'ability'
    ///   - npc.*/enc.*/spn.*/plc.* -> 'target_npc'
    ///
    /// One row per FQN occurrence in byte-position order. Dedup is by
    /// (hyd_fqn, ordinal) primary key. Returns total rows inserted.
    pub fn populate_hydra_refs(&self) -> Result<u64> {
        self.flush()?;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            fetch_fqn_payloads(&conn, "Hydra")?
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO hydra_refs (hyd_fqn, ordinal, ref_kind, ref_fqn) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (fqn, b64) in &rows {
                let Ok(payload) = BASE64.decode(b64) else {
                    continue;
                };
                let mut ordinal: i64 = 0;
                let mut seen_keys: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for ref_fqn in extract_hydra_fqn_refs(&payload) {
                    // Per-hyd dedup on the full ref_fqn — many hyd.* payloads
                    // carry each ref twice (length-prefix duplication artifact).
                    if !seen_keys.insert(ref_fqn.clone()) {
                        continue;
                    }
                    let kind = classify_hydra_ref(&ref_fqn);
                    stmt.execute(params![fqn, ordinal, kind, &ref_fqn])?;
                    inserted += 1;
                    ordinal += 1;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `quest_objectives` from each Quest payload (#216, supersedes
    /// the #130 MARKER_PRESENT foundation pass).
    ///
    /// Each quest payload encodes per-objective target references at CF40
    /// markers for property hash `5B20EAAA`. The value tail of each marker
    /// contains a class_ref to the target object (typically an `enc.*`
    /// encounter, `spn.*` spawn point, or `npc.*` NPC). The embedded
    /// reference uses one of three forms:
    ///
    ///   1. CF E0 content GUID -> resolves to objects.guid -> target FQN
    ///      (the form this populator extracts; ~38% of markers per quest)
    ///   2. CF 40 inline class ref -> points at an inline struct, no
    ///      separate GameObject; skipped
    ///   3. CC metadata reference -> bytes encode a CC-namespace ID, not
    ///      a content GUID; skipped
    ///
    /// One row per resolved 5B20EAAA marker. ordinal is the byte-position
    /// rank of the marker in the payload. kind is set to "target_ref" for
    /// resolved rows. count and name_string_id remain NULL pending decode
    /// of those per-objective fields (separate investigation).
    pub fn populate_quest_objectives(&self) -> Result<u64> {
        self.flush()?;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        const TARGET_HASH: [u8; 4] = [0x5B, 0x20, 0xEA, 0xAA];

        let payloads: Vec<(String, String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
            )?;
            let collected: Vec<(String, String, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };

        // Pre-load guid -> fqn lookup once for resolution.
        let guid_to_fqn: std::collections::HashMap<String, String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT guid, fqn FROM objects WHERE guid != ''")?;
            let collected: std::collections::HashMap<String, String> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO quest_objectives \
                    (quest_game_id, quest_fqn, ordinal, target_fqn, kind, count, name_string_id, raw_props) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL)",
            )?;
            for (game_id, fqn, b64) in &payloads {
                let Ok(payload) = BASE64.decode(b64) else {
                    continue;
                };
                let mut ordinal: i64 = 0;
                let mut i = 0;
                while i + 9 <= payload.len() {
                    let is_target = payload[i] == 0xCF
                        && payload[i + 1] == 0x40
                        && payload[i + 2] == 0x00
                        && payload[i + 3] == 0x00
                        && payload[i + 5..i + 9] == TARGET_HASH;
                    if !is_target {
                        i += 1;
                        continue;
                    }
                    // Decode the value tail looking for an embedded CF E0
                    // (content GUID reference). Format: `09 02 ?? CF E0 00
                    // <6-byte tail>` produces guid = "E000" + hex(tail).
                    let tail_end = (i + 9 + 30).min(payload.len());
                    let tail = &payload[i + 9..tail_end];
                    let mut target_fqn: Option<&str> = None;
                    if let Some(cfe0_off) = tail.windows(3).position(|w| w == [0xCF, 0xE0, 0x00]) {
                        let guid_tail_start = cfe0_off + 3;
                        if guid_tail_start + 6 <= tail.len() {
                            let guid_tail = &tail[guid_tail_start..guid_tail_start + 6];
                            let guid = format!("E000{}", hex::encode_upper(guid_tail));
                            target_fqn = guid_to_fqn.get(&guid).map(String::as_str);
                        }
                    }
                    if let Some(t) = target_fqn {
                        stmt.execute(params![game_id, fqn, ordinal, t, "target_ref"])?;
                        inserted += 1;
                    }
                    ordinal += 1;
                    i += 9;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Expand `quest_prerequisites` by scanning quest payload strings for the
    /// full set of SWTOR flag-style variable refs (#131, closes #67).
    ///
    /// `populate_quest_tables` only captures payload string refs starting
    /// with `has_`, which yields ~10 rows for 1,513 quests. This pass widens
    /// the prefix whitelist to the known SWTOR flag families to recover the
    /// rest of the story-arc graph:
    ///
    /// - `has_*`, `qstrew_*`, `qstv_*`, `cflag_*`, `glob_*`, `cdx_*`,
    ///   `ach_completed_*`, `completed_*`
    ///
    /// Each matching string is recorded verbatim as the `variable` column on
    /// the existing `quest_prerequisites(fqn, variable)` schema. Idempotent
    /// via `INSERT OR IGNORE`.
    pub fn populate_quest_prerequisites_graph(&self) -> Result<u64> {
        self.flush()?;

        const PREFIXES: &[&str] = &[
            "has_",
            "qstrew_",
            "qstv_",
            "cflag_",
            "glob_",
            "cdx_",
            "ach_completed_",
            "completed_",
        ];

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, json FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
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
        let mut inserted = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO quest_prerequisites (fqn, variable) VALUES (?1, ?2)",
            )?;
            for (fqn, json_str) in &rows {
                let v: serde_json::Value = match serde_json::from_str(json_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let strings = match v.get("strings").and_then(|s| s.as_array()) {
                    Some(s) => s,
                    None => continue,
                };
                for s_value in strings {
                    let s = match s_value.as_str() {
                        Some(s) => s,
                        None => continue,
                    };
                    if PREFIXES.iter().any(|p| s.starts_with(p)) {
                        let affected = stmt.execute(params![fqn, s])?;
                        if affected > 0 {
                            inserted += 1;
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `item_details` from every `kind = 'Item'` row by classifying
    /// the FQN. Mirrors `populate_quest_tables` in shape.
    /// Add direct quest -> npc edges by scanning each quest payload's strings
    /// for `npc.*` refs (#132, closes #48 #49).
    ///
    /// `populate_quest_npcs` joins quests to NPCs via the encounter+spawn
    /// graph. Planetary side quests often name their NPC directly in the
    /// quest payload without going through enc/spn intermediaries, so those
    /// rows never appear in `quest_npcs`. This pass picks up the direct case.
    /// Idempotent via `INSERT OR IGNORE`.
    pub fn populate_quest_npcs_direct(&self) -> Result<u64> {
        self.flush()?;

        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, json FROM objects WHERE kind = 'Quest' AND is_canonical = 1",
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
        let mut inserted = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO quest_npcs (quest_fqn, npc_fqn) VALUES (?1, ?2)",
            )?;
            for (fqn, json_str) in &rows {
                let v: serde_json::Value = match serde_json::from_str(json_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let strings = match v.get("strings").and_then(|s| s.as_array()) {
                    Some(s) => s,
                    None => continue,
                };
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for s_value in strings {
                    let s = match s_value.as_str() {
                        Some(s) => s,
                        None => continue,
                    };
                    if s.starts_with("npc.") && seen.insert(s.to_string()) {
                        let affected = stmt.execute(params![fqn, s])?;
                        if affected > 0 {
                            inserted += 1;
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
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
}

/// Classify a quest FQN into zero or more (cluster_kind, cluster_id) pairs.
///
/// One quest can populate multiple cluster rows because the FQN encodes
/// orthogonal axes (e.g. a class_act bucket plus an expansion bucket).
pub(crate) fn classify_quest_clusters(fqn: &str) -> Vec<(&'static str, String)> {
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

/// Extract FQN-shaped ASCII strings from a Hydra payload's bytes.
/// Hydra scripts encode their references as length-prefixed ASCII
/// strings (each preceded by a single length byte). This scanner finds
/// runs of printable ASCII >= 8 chars that match a SWTOR FQN prefix.
///
/// Returns refs in byte-position order. Per-payload caller deduplicates.
pub(crate) fn extract_hydra_fqn_refs(payload: &[u8]) -> Vec<String> {
    const FQN_PREFIXES: &[&str] = &[
        "qst.", "cnv.", "abl.", "npc.", "enc.", "spn.", "plc.", "hyd.", "mpn.",
    ];
    let mut out: Vec<String> = Vec::new();
    let mut emit_from = |buf: &[u8]| {
        if buf.len() < 8 {
            return;
        }
        let Ok(s) = std::str::from_utf8(buf) else {
            return;
        };
        for prefix in FQN_PREFIXES {
            let Some(idx) = s.find(prefix) else { continue };
            // Filter out garbage where the prefix appears mid-token
            // (e.g. previous char is alphanumeric/underscore/dot).
            if idx > 0 && is_fqn_char(s.as_bytes()[idx - 1]) {
                return;
            }
            out.push(s[idx..].to_string());
            return;
        }
    };
    let mut buf: Vec<u8> = Vec::new();
    for &b in payload {
        if (0x20..0x7F).contains(&b) {
            buf.push(b);
        } else {
            emit_from(&buf);
            buf.clear();
        }
    }
    emit_from(&buf);
    out
}

/// Classify a Hydra-referenced FQN by its prefix and suffix family.
pub(crate) fn classify_hydra_ref(fqn: &str) -> &'static str {
    if let Some(rest) = fqn.strip_prefix("qst.") {
        // qst.<path>.<final_segment> -- inspect final segment family
        let final_seg = rest.rsplit('.').next().unwrap_or(rest);
        if final_seg.starts_with("counter_") {
            return "counter";
        }
        if final_seg.starts_with("track_") {
            return "tracking";
        }
        if final_seg.starts_with("jrn_") {
            return "journal";
        }
        if final_seg.starts_with("qm_") {
            return "qm_state";
        }
        if final_seg.starts_with("cnv_") {
            return "cnv_flag";
        }
        if final_seg.starts_with("glb_") {
            return "glb_flag";
        }
        if final_seg.starts_with("hook_") {
            return "hook";
        }
        // A bare qst.<path>.<name> with no recognized flag prefix and many
        // segments is the quest root reference. Heuristic: short suffix
        // (no underscore) tends to be a quest name segment.
        if !rest.contains('.') {
            return "quest_self";
        }
        return "other_flag";
    }
    if fqn.starts_with("cnv.") {
        return "conversation";
    }
    if fqn.starts_with("abl.") {
        return "ability";
    }
    if fqn.starts_with("npc.")
        || fqn.starts_with("enc.")
        || fqn.starts_with("spn.")
        || fqn.starts_with("plc.")
    {
        return "target_npc";
    }
    if fqn.starts_with("hyd.") {
        return "hydra";
    }
    if fqn.starts_with("mpn.") {
        return "mission_point";
    }
    "other"
}

/// Categorize a SWTOR quest progression-flag string by its leading
/// underscore-delimited segment.
///
/// Categories mirror the conventions seen across the quest corpus:
/// - `jrn`: journal-display trigger (e.g. `jrn_start_speak_to_unaw_aharo`)
/// - `track`: explicit progress tracker (`track_defeat_callef`)
/// - `hook`: hydra script event (`hook_holo_triggered_temple`)
/// - `qm`: quest-manager state (`qm_flesh_raiders_killed`)
/// - `counter`: kill/gather counter (`counter_flesh_raiders`)
/// - `hyd`: hydra runtime event (`hyd_raider_wave_1`)
/// - `branch_step`: the `_bN_sN_tN` per-step coordinate
/// - `quest_reward`: reward variable (`quest_reward_01`)
/// - `spoke`: action-specific completion (`spoke_to_satele_via_holo`)
/// - `cnv`: conversation-set flag
/// - `glb`: global story flag
/// - `complex`: multi-condition flag
/// - `other`: anything else
pub(crate) fn classify_quest_flag(name: &str) -> &'static str {
    // Branch/step/tier form: leading underscore then b<N>_s<N>_t<N>
    if let Some(rest) = name.strip_prefix("_b") {
        if let Some(rest) = rest.split_once('_').and_then(|(_, r)| r.strip_prefix('s')) {
            if let Some((_, r)) = rest.split_once('_') {
                if r.starts_with('t') {
                    return "branch_step";
                }
            }
        }
    }
    match name.split_once('_').map(|(p, _)| p).unwrap_or(name) {
        "jrn" => "jrn",
        "track" => "track",
        "hook" => "hook",
        "qm" => "qm",
        "counter" => "counter",
        "hyd" => "hyd",
        "quest" if name.starts_with("quest_reward") => "quest_reward",
        "spoke" => "spoke",
        "cnv" => "cnv",
        "glb" => "glb",
        "complex" => "complex",
        _ => "other",
    }
}

/// Count quest steps by looking for branch/step/task patterns in payload strings.
/// Pattern: `_bX_sY_tZ` where X=branch, Y=step, Z=task.
pub(crate) fn count_quest_steps(json_str: &str) -> i32 {
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

/// Create the quest/mission/chain/cluster tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
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
            -- Quest milestones (#265): the qm_*/go_* declaration a quest sets on
            -- completion -- its "I'm done" signal. Derived from quest_objective_flags
            -- (no payload re-decode). is_terminal=1 marks the byte-order-last qm/go
            -- flag per quest, the unambiguous completion-detection target kessel-warden
            -- matches against the live combat log. Cross-quest prereq matching is NOT
            -- done here -- those variables are internal per-quest state (prereq_finding.md).
            CREATE TABLE IF NOT EXISTS quest_milestones (
                quest_fqn       TEXT NOT NULL,
                milestone_name  TEXT NOT NULL,
                is_terminal     INTEGER NOT NULL,
                PRIMARY KEY (quest_fqn, milestone_name)
            );
            CREATE INDEX IF NOT EXISTS idx_quest_milestones_quest ON quest_milestones(quest_fqn);
            CREATE INDEX IF NOT EXISTS idx_quest_milestones_terminal ON quest_milestones(is_terminal);
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
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::temp_db_path;
    #[test]
    fn migrate_quest_typed_columns_adds_columns_idempotently() {
        let path = temp_db_path("quest_typed_migrate");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        // Re-run should be a no-op (idempotency check).
        db.migrate_quest_typed_columns().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(quest_details)").unwrap();
        let names: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for col in [
            "activity_type",
            "difficulty",
            "rewards_visibility",
            "episode_season",
            "level",
        ] {
            assert!(names.contains(col), "missing typed column {col}: {names:?}");
        }
    }
    #[test]
    fn populate_quest_details_typed_decodes_real_enum_value() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let path = temp_db_path("quest_typed_pop");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        let (activity_hi32, expected_member) = {
            let mut found: Option<(u32, String)> = None;
            let class = crate::gom_schema::class_for_type_hi32(QUEST_CLASS_TYPE_HI32);
            if let Some(c) = class {
                for prop_ref in &c.property_refs {
                    if prop_ref.len() < 16 {
                        continue;
                    }
                    let low32_hex = &prop_ref[8..16];
                    if let Ok(hi32) = u32::from_str_radix(low32_hex, 16) {
                        if let Some(prop) = crate::gom_schema::property_for_cf40(hi32) {
                            if let Some(refs) = &prop.refs {
                                if let Some(r) =
                                    refs.iter().find(|r| r.name.contains("qstActivityType"))
                                {
                                    // Grab the enum's first member for our expected value
                                    let mem = crate::gom_schema::enum_for_name(&r.name)
                                        .and_then(|e| e.members.first().cloned())
                                        .unwrap_or_default();
                                    found = Some((hi32, mem));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            found.unwrap_or((0, String::new()))
        };
        if activity_hi32 == 0 || expected_member.is_empty() {
            return;
        }
        // Real CF40 marker + value: 05 00 = enum_ref index 0
        let mut payload = vec![0u8; 4];
        payload.push(0xCF);
        payload.push(0x40);
        payload.extend_from_slice(&[0u8; 2]);
        payload.push(0x40);
        payload.extend_from_slice(&activity_hi32.to_be_bytes());
        payload.push(0x05); // enum_ref tag
        payload.push(0x00); // enum index 0
        let payload_b64 = BASE64.encode(&payload);

        {
            let conn = db.conn.lock().unwrap();
            let json = format!(r#"{{"payload_b64":"{payload_b64}"}}"#);
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json, is_canonical) \
                 VALUES ('qt1', 'sid1', 'ph1', 'guid1', 'qst.test.example', 'Quest', ?1, 1)",
                params![json],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO quest_details (fqn, mission_type) VALUES ('qst.test.example', 'unknown')",
                [],
            )
            .unwrap();
        }
        let updated = db.populate_quest_details_typed().unwrap();
        assert_eq!(updated, 1);
        let conn = db.conn.lock().unwrap();
        let activity: Option<String> = conn
            .query_row(
                "SELECT activity_type FROM quest_details WHERE fqn = 'qst.test.example'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(activity.as_deref(), Some(expected_member.as_str()));
    }
    #[test]
    fn populate_quest_details_typed_resolves_heroic_activity_by_hash() {
        // Locks in the #271 subsumption: a quest whose qstActivityType enum
        // decodes to index 2 must classify as the Heroic member, resolved via
        // the canonical hash path (B58CE5FC) -- not the FQN/name (the heroic
        // signal is the payload enum, not the title).
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let expected = crate::gom_schema::enum_for_name("qstActivityType")
            .and_then(|e| e.members.get(2).cloned());
        let Some(expected) = expected else {
            return; // schema lacks the enum -> nothing to assert
        };
        assert!(
            expected.contains("Heroic"),
            "member index 2 should be the Heroic activity type, got {expected}"
        );
        let path = temp_db_path("quest_typed_heroic");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // CF40 marker for hash B58CE5FC + enum_ref tag (0x05) + index 2.
        let hi32: u32 = 0xB58C_E5FC;
        let mut payload = vec![0u8; 4];
        payload.push(0xCF);
        payload.push(0x40);
        payload.extend_from_slice(&[0u8; 2]);
        payload.push(0x40);
        payload.extend_from_slice(&hi32.to_be_bytes());
        payload.push(0x05);
        payload.push(0x02);
        let payload_b64 = BASE64.encode(&payload);
        {
            let conn = db.conn.lock().unwrap();
            let json = format!(r#"{{"payload_b64":"{payload_b64}"}}"#);
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json, is_canonical) \
                 VALUES ('qh1', 'sid', 'ph', 'g', 'qst.test.heroic', 'Quest', ?1, 1)",
                params![json],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO quest_details (fqn, mission_type) VALUES ('qst.test.heroic', 'side')",
                [],
            )
            .unwrap();
        }
        db.populate_quest_details_typed().unwrap();
        let conn = db.conn.lock().unwrap();
        let activity: Option<String> = conn
            .query_row(
                "SELECT activity_type FROM quest_details WHERE fqn = 'qst.test.heroic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(activity.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn quest_objectives_table_exists_after_init() {
        let path = temp_db_path("quest_obj_table");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quest_objectives'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "quest_objectives table missing after init");
    }
    #[test]
    fn populate_quest_objectives_handles_empty_archive() {
        let path = temp_db_path("quest_obj_empty");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let n = db.populate_quest_objectives().unwrap();
        assert_eq!(n, 0);
    }
    #[test]
    fn populate_quest_prerequisites_graph_extracts_flag_families() {
        let path = temp_db_path("quest_prereqs_graph");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let json = serde_json::json!({
            "strings": [
                "has_completed_intro",
                "qstrew_lord_grathan_xp",
                "qstv_act1_state",
                "cflag_dark_choice",
                "glob_world_event",
                "cdx_lore_unlock",
                "ach_completed_kill_boss",
                "completed_phase_1",
                "ignored_random_string",
                "qst.unrelated.fqn",
            ]
        })
        .to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json, is_canonical) \
                 VALUES ('q1', 'sid', 'ph', 'g1', 'qst.test.example', 'Quest', ?1, 1)",
                params![json],
            )
            .unwrap();
        }
        let n = db.populate_quest_prerequisites_graph().unwrap();
        assert_eq!(n, 8, "should capture all 8 flag-family prefixes");
        let conn = db.conn.lock().unwrap();
        let variables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT variable FROM quest_prerequisites WHERE fqn = 'qst.test.example' ORDER BY variable")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert!(variables.contains(&"qstrew_lord_grathan_xp".to_string()));
        assert!(variables.contains(&"cdx_lore_unlock".to_string()));
        assert!(!variables.iter().any(|v| v == "ignored_random_string"));
    }
    #[test]
    fn populate_quest_prerequisites_graph_is_idempotent() {
        let path = temp_db_path("quest_prereqs_idem");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let json = serde_json::json!({"strings": ["has_one", "qstv_two"]}).to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json, is_canonical) \
                 VALUES ('q1', 'sid', 'ph', 'g1', 'qst.test.idem', 'Quest', ?1, 1)",
                params![json],
            )
            .unwrap();
        }
        let first = db.populate_quest_prerequisites_graph().unwrap();
        let second = db.populate_quest_prerequisites_graph().unwrap();
        assert_eq!(first, 2);
        assert_eq!(
            second, 0,
            "INSERT OR IGNORE should produce zero affected on re-run"
        );
    }
    #[test]
    fn populate_quest_npcs_direct_picks_up_inline_refs() {
        let path = temp_db_path("quest_npcs_direct");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let json = serde_json::json!({
            "strings": [
                "npc.korriban.overseer_ragate",
                "npc.korriban.acolyte_grunt",
                "qst.unrelated.fqn",
                "npc.korriban.overseer_ragate", // duplicate must dedupe
                "spn.korriban.spawn_table",
            ]
        })
        .to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json, is_canonical) \
                 VALUES ('q1', 'sid', 'ph', 'g1', 'qst.korriban.side_a', 'Quest', ?1, 1)",
                params![json],
            )
            .unwrap();
        }
        let n = db.populate_quest_npcs_direct().unwrap();
        assert_eq!(n, 2, "should insert 2 unique npc.* edges");
        let count: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM quest_npcs WHERE quest_fqn = 'qst.korriban.side_a'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 2);
        // Re-run is idempotent
        let n2 = db.populate_quest_npcs_direct().unwrap();
        assert_eq!(n2, 0);
    }

    /// Seed quest_objective_flags rows for the milestone tests.
    fn seed_flag(db: &Database, quest_fqn: &str, ordinal: i64, flag: &str, cat: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quest_objective_flags (quest_fqn, ordinal, flag_name, flag_category) \
             VALUES (?1, ?2, ?3, ?4)",
            params![quest_fqn, ordinal, flag, cat],
        )
        .unwrap();
    }

    #[test]
    fn populate_quest_milestones_marks_exactly_one_terminal_per_quest_with_qm() {
        let path = temp_db_path("quest_milestones_terminal");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        // qm_started(0), qm_phase2(2), go_final(3, the byte-order-last qm/go).
        // track/counter rows are not milestones and must be ignored, even
        // though counter_y(4) has a higher ordinal than the terminal milestone.
        let q = "qst.test.milestone";
        seed_flag(&db, q, 0, "qm_started", "qm");
        seed_flag(&db, q, 1, "track_progress", "track");
        seed_flag(&db, q, 2, "qm_phase2", "qm");
        seed_flag(&db, q, 3, "go_final", "other");
        seed_flag(&db, q, 4, "counter_kills", "counter");

        let n = db.populate_quest_milestones().unwrap();
        assert_eq!(n, 3, "qm_started, qm_phase2, go_final are the 3 milestones");

        let conn = db.conn.lock().unwrap();
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM quest_milestones WHERE quest_fqn = ?1",
                params![q],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, 3);
        let terminals: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM quest_milestones WHERE quest_fqn = ?1 AND is_terminal = 1",
                params![q],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminals, 1, "exactly one terminal milestone per quest");
    }

    #[test]
    fn quest_milestones_terminal_matches_last_qm_by_ordinal() {
        let path = temp_db_path("quest_milestones_lastord");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let q = "qst.test.lastord";
        seed_flag(&db, q, 0, "qm_started", "qm");
        seed_flag(&db, q, 2, "qm_phase2", "qm");
        seed_flag(&db, q, 3, "go_final", "other");
        db.populate_quest_milestones().unwrap();

        let conn = db.conn.lock().unwrap();
        let terminal: String = conn
            .query_row(
                "SELECT milestone_name FROM quest_milestones \
                 WHERE quest_fqn = ?1 AND is_terminal = 1",
                params![q],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            terminal, "go_final",
            "terminal milestone is the max-ordinal qm/go flag"
        );
    }

    #[test]
    fn quest_milestones_terminal_is_isolated_per_quest() {
        // The correlated subquery scopes the per-quest max ordinal with
        // WHERE g.quest_fqn = f.quest_fqn. Prove one quest's higher ordinal
        // does not steal the terminal flag from another quest in the same DB.
        let path = temp_db_path("quest_milestones_isolation");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        // Quest A terminal at ordinal 5; Quest B terminal at ordinal 2 (< A's max).
        seed_flag(&db, "qst.a", 0, "qm_start", "qm");
        seed_flag(&db, "qst.a", 5, "qm_done", "qm");
        seed_flag(&db, "qst.b", 0, "qm_start", "qm");
        seed_flag(&db, "qst.b", 2, "qm_done", "qm");
        db.populate_quest_milestones().unwrap();

        let conn = db.conn.lock().unwrap();
        let b_terminal: String = conn
            .query_row(
                "SELECT milestone_name FROM quest_milestones \
                 WHERE quest_fqn = 'qst.b' AND is_terminal = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(b_terminal, "qm_done");
        let total_terminals: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM quest_milestones WHERE is_terminal = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total_terminals, 2, "one terminal per quest, isolated");
    }

    #[test]
    fn populate_quest_milestones_is_idempotent() {
        let path = temp_db_path("quest_milestones_idem");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let q = "qst.test.idem";
        seed_flag(&db, q, 0, "qm_started", "qm");
        seed_flag(&db, q, 1, "go_done", "other");

        let first = db.populate_quest_milestones().unwrap();
        let second = db.populate_quest_milestones().unwrap();
        assert_eq!(first, 2);
        assert_eq!(second, 2, "INSERT OR REPLACE rewrites the same 2 rows");

        // Idempotent state: row count and the single terminal are unchanged.
        let conn = db.conn.lock().unwrap();
        let (total, terminals): (i64, i64) = conn
            .query_row(
                "SELECT \
                    (SELECT COUNT(*) FROM quest_milestones WHERE quest_fqn = ?1), \
                    (SELECT COUNT(*) FROM quest_milestones WHERE quest_fqn = ?1 AND is_terminal = 1)",
                params![q],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(terminals, 1);
    }
}
