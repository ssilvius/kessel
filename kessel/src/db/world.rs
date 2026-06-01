//! World/spatial extraction: appearance, fx, scripts, nodes, planets, spawns, conversation refs.

use super::*;

impl Database {
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
}

/// Convert a `.fxspec` resource path into the path-relative key used by
/// `<fxSpecString>` references in `.epp` files. Returns None when the
/// path doesn't contain a `/fxspec/` segment or a `.fxspec` suffix. Used
/// by `populate_fx_specs` (#183) to make `appearance_specs.fx_spec_refs`
/// joinable to `fx_specs.fqn`.
///
/// Example: `/resources/art/fx/fxspec/abilities/sith_warrior/sw_massacre_sword_glow.fxspec`
/// → `abilities/sith_warrior/sw_massacre_sword_glow`.
pub(crate) fn fxspec_fqn_from_path(path: &str) -> Option<String> {
    let after_marker = path.split_once("/fxspec/")?.1;
    let trimmed = after_marker.strip_suffix(".fxspec")?;
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Search a record's bytes for the CC <cc_id> marker, then extract the
/// trailing length-prefixed `_pla_<planet>` ASCII string. Used by the
/// conquest events populator.
pub(crate) fn find_planet_code_after_cc(record: &[u8], cc_id: &[u8; 4]) -> Option<String> {
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

/// Create the world tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
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
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
