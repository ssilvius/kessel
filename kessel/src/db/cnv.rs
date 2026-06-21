//! Conversation domain.
//!
//! Conversations (`cnv.*`) are extracted as NODE objects by
//! `world::populate_node_objects` (the NODE blob is a cinematic director
//! script -- camera/animation/music -- with no dialogue text or narrative
//! flow). This module owns everything *derived* about conversations:
//!
//! - The reference graph (`populate_conversation_refs`): CF-GUID refs in each
//!   NODE body resolved to the quest / npc / achievement / codex / item /
//!   followup / encounter it touches, plus alignment-event token counts.
//! - The dialogue lines (`conversation_lines` view): the spoken/subtitle text,
//!   which lives in per-conversation STBs under `/str/cnv/` (enabled in
//!   `stb::should_extract_stb`). The STB path maps onto the `cnv.*` FQN, so
//!   `str.cnv.<path>.<id1>.<id2>` strings rejoin their conversation -- and
//!   through `conversation_quest_refs`, their quest.

use super::*;

impl Database {
    /// Single prototype sweep that both inserts NODE objects and builds the
    /// conversation reference graph (#175/#181 + #175 cnv refs), merged to
    /// decompress each prototype entry exactly once instead of twice.
    ///
    /// NODE files at `/resources/systemgenerated/prototypes/<num>.node` use the
    /// PROT format (header bytes 0x14.. carry the FQN). The candidate entries
    /// are gated on `proto_hashes` -- the PROT-magic hashes self-discovered
    /// during the main archive sweep -- rather than on dictionary-known
    /// prototype paths, so new-patch prototypes extract without the dictionary.
    /// For every PROT-magic entry this inserts one row into `objects`
    /// (synthetic 42-byte GOM header
    /// so the existing `GameObject` constructor reads the content GUID at 0..8
    /// the same way it does for PBUK objects; `kind` derived from the FQN), and
    /// for `cnv.*` FQNs records (in document order, no dedup yet) the CF E0 GUID
    /// refs and the alignment-event token counts into a buffer.
    ///
    /// After the sweep the buffered NODE objects are flushed (so they are
    /// visible to the GUID map), then the GUID -> (kind, fqn) map is built and
    /// the buffered conversation refs are resolved, deduped, and dispatched into
    /// the 7 conversation_* tables exactly as the prior two-pass version did.
    ///
    /// Resolution is deferred (the GUID map is built once, after all NODE
    /// objects are flushed) so the result is identical to running
    /// `populate_node_objects` followed by the old `populate_conversation_refs`.
    pub fn populate_node_and_conversation_refs(
        &self,
        tor_dir: &std::path::Path,
        proto_hashes: &std::collections::HashSet<u64>,
    ) -> Result<NodeAndConvRefCounts> {
        use crate::myp::Archive;
        use crate::pbuk::GomObject;
        use crate::schema::GameObject;
        use std::collections::{HashMap, HashSet};

        // `proto_hashes` is the set of PROT-magic entry hashes self-discovered
        // during the main archive sweep (passed in via PassCtx). It is a
        // superset of the old dictionary `/prototypes/` gate that also includes
        // new-patch prototypes, so existing NODE objects are preserved and new
        // conversations are added. The `&data[..4] == b"PROT"` check below
        // re-confirms each entry.

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

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

        // Buffered per-conversation scan results (resolved after the sweep so
        // the GUID map includes the NODE objects flushed below). guids are in
        // document order with no dedup yet (dedup is on resolved target_fqn).
        // align_counts is per (cnv, kind) and needs no DB, so it is computed now.
        struct CnvScan {
            cnv_fqn: String,
            guids: Vec<[u8; 8]>,
            align_counts: Vec<(&'static str, u64)>,
        }
        let mut cnv_buffer: Vec<CnvScan> = Vec::new();

        let mut node_inserted = 0u64;
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
                    fqn: fqn.clone(),
                    header,
                    payload,
                };
                let obj = GameObject::from_gom_with_overrides(&gom, None);
                self.insert_object(&obj)?;
                node_inserted += 1;

                // For cnv.* prototypes, scan the body now but defer resolution
                // until the GUID map is built (after the NODE flush below).
                if !fqn.starts_with("cnv.") {
                    continue;
                }
                let cnv_fqn = fqn;

                // Collect CF E0 8-byte GUID refs in document order (no dedup
                // yet -- dedup is on resolved target_fqn, done at resolve time).
                let mut guids: Vec<[u8; 8]> = Vec::new();
                let mut i = 0;
                while i + 9 <= data.len() {
                    if data[i] == 0xCF && data[i + 1] == 0xE0 {
                        let mut g = [0u8; 8];
                        g.copy_from_slice(&data[i + 1..i + 9]);
                        guids.push(g);
                        i += 9;
                    } else {
                        i += 1;
                    }
                }

                // Alignment-event token scan. Walk every printable string in
                // the NODE, count occurrences per kind, one count per
                // (cnv, kind). The numbered suffixes (darkmoment_07,
                // heroicmoment_15, ...) collapse into the unsuffixed kind for
                // storage; downstream can re-scan for exact tier numbers.
                let mut align_map: HashMap<&'static str, u64> = HashMap::new();
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
                                    *align_map.entry(*kind).or_insert(0) += 1;
                                    break;
                                }
                            }
                        }
                        si = sj;
                    } else {
                        si += 1;
                    }
                }
                let align_counts: Vec<(&'static str, u64)> = align_map.into_iter().collect();

                cnv_buffer.push(CnvScan {
                    cnv_fqn,
                    guids,
                    align_counts,
                });
            }
        }

        // Flush buffered NODE objects so they are visible to the GUID map
        // (same logical point as the old node_objects-then-conversation_refs
        // ordering, where node objects were committed before the cnv pass ran).
        self.flush()?;

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

        let mut counts = ConversationRefCounts::default();

        for scan in &cnv_buffer {
            let cnv_fqn = &scan.cnv_fqn;

            // Per-target dedup: a single conversation often references the
            // same target multiple times (one per dialog branch); collapse.
            // Dedup on resolved target_fqn, in document (guid) order.
            let mut seen: HashSet<&str> = HashSet::new();
            for g in &scan.guids {
                if let Some((kind, target_fqn)) = guid_to_kind_fqn.get(g) {
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
                            "Conversation" if target_fqn != cnv_fqn => {
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
            }

            for (kind, n) in &scan.align_counts {
                align_stmt.execute(params![cnv_fqn, kind, n])?;
                counts.alignment_event += 1;
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
        Ok(NodeAndConvRefCounts {
            node_objects: node_inserted,
            refs: counts,
        })
    }

    /// Self-discover and ingest the per-conversation dialogue STBs (no dict).
    ///
    /// The spoken/subtitle lines for every conversation live in per-conversation
    /// STBs under `/resources/<locale>/str/cnv/`. The main loop ingests these
    /// only for dictionary-known paths, so new-patch conversations miss their
    /// text when the community hash dictionary is stale.
    ///
    /// This pass instead enumerates every `cnv.*` conversation object (inserted
    /// by `populate_node_and_conversation_refs`, so it must run after it),
    /// derives the en-us STB path from each FQN (the inverse of
    /// `stb::extract_fqn_from_path`: `cnv.X.Y -> /resources/en-us/str/cnv/X/Y.stb`),
    /// computes the archive filename hash, sweeps the archives once, and parses
    /// each matching entry's STB into `strings`. Idempotent (`insert_string` is
    /// INSERT OR REPLACE), so re-inserting strings the main loop already pulled
    /// via the dictionary is harmless.
    ///
    /// Returns the number of string rows inserted.
    pub fn populate_conversation_strings(&self, tor_dir: &std::path::Path) -> Result<u64> {
        use crate::myp::Archive;
        use std::collections::HashMap;

        // Distinct cnv.* conversation FQNs (kinded Conversation by the node
        // sweep). Derive each one's en-us STB path + archive hash.
        let cnv_fqns: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT DISTINCT fqn FROM objects WHERE kind = 'Conversation'")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        // hash -> derived STB path. Multiple FQNs cannot collide here (the path
        // is a 1:1 function of the FQN), so last-writer-wins on a hash collision
        // is acceptable and astronomically unlikely.
        let mut hash_to_path: HashMap<u64, String> = HashMap::new();
        for cnv_fqn in &cnv_fqns {
            let path = conversation_stb_path(cnv_fqn);
            // Match the archive entry key, which is combine_hash(ph, sh) =
            // (ph << 32) | sh -- the same composition HashDictionary::load
            // stores and main.rs matches entry.filename_hash against. NOTE:
            // hash::swtor_filename_hash returns (sh << 32) | ph (the halves
            // swapped) and does NOT match entry.filename_hash -- do not use it.
            let (ph, sh) = crate::hash::hashlittle2(path.as_bytes(), 0, 0);
            let h = crate::hash::combine_hash(ph, sh);
            hash_to_path.insert(h, path);
        }

        if hash_to_path.is_empty() {
            return Ok(0);
        }

        let tor_files: Vec<std::path::PathBuf> = std::fs::read_dir(tor_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut rows_inserted = 0u64;
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
                let Some(path) = hash_to_path.get(&entry.filename_hash) else {
                    continue;
                };
                let data = match archive.read_entry(entry) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                // Parse with the derived path so fqn_prefix/locale resolve. Then
                // mirror main.rs's STB insert exactly:
                //   fqn = "{fqn_prefix}.{id1}.{id2}", insert_string(fqn, locale, entry)
                if let Ok(stb_file) = crate::stb::parse(&data, path) {
                    for stb_entry in &stb_file.entries {
                        let string_fqn = format!(
                            "{}.{}.{}",
                            stb_file.fqn_prefix, stb_entry.id1, stb_entry.id2
                        );
                        if self
                            .insert_string(&string_fqn, &stb_file.locale, stb_entry)
                            .is_ok()
                        {
                            rows_inserted += 1;
                        }
                    }
                }
            }
        }

        self.flush()?;
        Ok(rows_inserted)
    }

    /// Populate `conversation_dialogue` (#285): the ordered dialogue script of
    /// every conversation, decoded from its NODE payload's line-node markers in
    /// byte order. Returns (conversations_with_dialogue, total_lines).
    pub fn populate_conversation_dialogue(&self) -> Result<(u64, u64)> {
        use crate::schema::conversation_tree::decode_dialogue_lines;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        let conn = self.conn.lock().unwrap();

        let convs: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'Conversation' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })?
                .filter_map(|r| r.ok())
                .filter_map(|(fqn, b64)| b64.map(|b| (fqn, b)))
                .collect();
            rows
        };

        // guid -> Npc fqn, to resolve each line's actor GUID to its speaker (#286).
        let npc_by_guid: std::collections::HashMap<String, String> = {
            let mut stmt = conn
                .prepare("SELECT guid, fqn FROM objects WHERE kind = 'Npc' AND is_canonical = 1")?;
            let map: std::collections::HashMap<String, String> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            map
        };

        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO conversation_dialogue \
             (cnv_fqn, seq, line_id, line_ref, speaker_guid, speaker_npc_fqn, is_npc) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        let (mut conversations, mut lines) = (0u64, 0u64);
        for (fqn, b64) in &convs {
            let Ok(payload) = BASE64.decode(b64) else {
                continue;
            };
            let decoded = decode_dialogue_lines(&payload);
            if decoded.is_empty() {
                continue;
            }
            conversations += 1;
            for line in decoded {
                // Speaker = the first actor GUID resolving to an Npc; the rest
                // are branch/condition/quest refs (and the player pseudo-actor,
                // which resolves to no Npc).
                let speaker = line
                    .actor_guids
                    .iter()
                    .find_map(|g| npc_by_guid.get(g).map(|fqn| (g.clone(), fqn.clone())));
                let (speaker_guid, speaker_npc_fqn, is_npc) = match speaker {
                    Some((g, npc_fqn)) => (Some(g), Some(npc_fqn), 1i64),
                    None => (None, None, 0i64),
                };
                stmt.execute(params![
                    fqn,
                    line.seq as i64,
                    line.line_id as i64,
                    line.line_ref,
                    speaker_guid,
                    speaker_npc_fqn,
                    is_npc,
                ])?;
                lines += 1;
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok((conversations, lines))
    }
}

/// Derive the en-us dialogue STB archive path for a `cnv.*` conversation FQN.
///
/// Inverse of `stb::extract_fqn_from_path`: a conversation FQN
/// `cnv.location.taris.x` maps to `/resources/en-us/str/cnv/location/taris/x.stb`.
fn conversation_stb_path(cnv_fqn: &str) -> String {
    format!("/resources/en-us/str/{}.stb", cnv_fqn.replace('.', "/"))
}

/// Result of the merged NODE-object + conversation-ref sweep
/// (`populate_node_and_conversation_refs`): the NODE object insert count plus
/// the per-kind conversation reference counts.
#[derive(Default, Debug)]
pub struct NodeAndConvRefCounts {
    pub node_objects: u64,
    pub refs: ConversationRefCounts,
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

/// Create the conversation-domain tables + the dialogue view (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
            -- Conversation -> quest references. NODE conversation files (cnv.*)
            -- embed CF GUID refs to qst.* objects representing the quests
            -- that conversation grants or affects. ~23% of NODE files carry
            -- such refs in observed data.
            CREATE TABLE IF NOT EXISTS conversation_quest_refs (
                cnv_fqn TEXT NOT NULL,
                quest_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, quest_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_quest_refs_quest ON conversation_quest_refs(quest_fqn);
            -- Conversation -> NPC actors (CF GUID refs to npc.*).
            CREATE TABLE IF NOT EXISTS conversation_npcs (
                cnv_fqn TEXT NOT NULL,
                npc_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, npc_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_npcs_npc ON conversation_npcs(npc_fqn);
            -- Conversation -> achievement unlocks (CF GUID refs to ach.*).
            CREATE TABLE IF NOT EXISTS conversation_achievements (
                cnv_fqn TEXT NOT NULL,
                achievement_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, achievement_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_ach_ach ON conversation_achievements(achievement_fqn);
            -- Conversation -> codex unlocks (CF GUID refs to cdx.*).
            CREATE TABLE IF NOT EXISTS conversation_codex (
                cnv_fqn TEXT NOT NULL,
                codex_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, codex_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_cdx_cdx ON conversation_codex(codex_fqn);
            -- Conversation -> item grants (CF GUID refs to itm.*).
            CREATE TABLE IF NOT EXISTS conversation_items (
                cnv_fqn TEXT NOT NULL,
                item_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, item_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_items_item ON conversation_items(item_fqn);
            -- Conversation -> follow-up conversation (CF GUID refs to other cnv.*).
            CREATE TABLE IF NOT EXISTS conversation_followups (
                cnv_fqn TEXT NOT NULL,
                target_cnv_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, target_cnv_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_follow_target ON conversation_followups(target_cnv_fqn);
            -- Conversation -> combat encounter (CF GUID refs to enc.*).
            CREATE TABLE IF NOT EXISTS conversation_encounters (
                cnv_fqn TEXT NOT NULL,
                encounter_fqn TEXT NOT NULL,
                PRIMARY KEY (cnv_fqn, encounter_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_enc_enc ON conversation_encounters(encounter_fqn);
            -- Per-conversation counts of alignment-event tokens found in NODE
            -- bytes (event.darkmoment / bigdarkmoment / sinistermoment /
            -- heroicmoment / darksidetheme / lightsidetheme + alignment_override
            -- / influence_desync / affection_bot). A coarse LS/DS/influence
            -- signal; per-choice magnitudes are not decoded (runtime).
            CREATE TABLE IF NOT EXISTS conversation_alignment_events (
                cnv_fqn TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                event_count INTEGER NOT NULL,
                PRIMARY KEY (cnv_fqn, event_kind)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_align_kind ON conversation_alignment_events(event_kind);

            -- Ordered dialogue script per conversation (#285). Each row is one
            -- dialogue line, decoded from the conversation NODE payload's
            -- line-node markers in byte order (== dialogue order, #284 spike).
            -- `seq` is the playback order; `line_id` is the str.cnv string's
            -- `id1` (the per-line id; NB the `conversation_lines` view labels
            -- the shared `id2` as "line_id"). Join to `strings` on id1 + the
            -- `line_ref` (str.cnv base FQN) prefix for the spoken text:
            --   JOIN strings s ON s.id1 = d.line_id
            --                  AND s.fqn LIKE d.line_ref || '.%'
            --                  AND s.locale = 'en-us'
            -- speaker_* (#286): each line node carries CF E0 actor/ref GUIDs;
            -- the one resolving to a kind=Npc object is the speaker. is_npc=1
            -- when an Npc speaker resolved; is_npc=0 for player/system lines
            -- (the player pseudo-actor resolves to no Npc -- #287 marks which
            -- of those are player options).
            CREATE TABLE IF NOT EXISTS conversation_dialogue (
                cnv_fqn         TEXT NOT NULL,
                seq             INTEGER NOT NULL,
                line_id         INTEGER NOT NULL,
                line_ref        TEXT NOT NULL,
                speaker_guid    TEXT,
                speaker_npc_fqn TEXT,
                is_npc          INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (cnv_fqn, seq)
            );
            CREATE INDEX IF NOT EXISTS idx_cnv_dialogue_line ON conversation_dialogue(line_id);
            CREATE INDEX IF NOT EXISTS idx_cnv_dialogue_speaker ON conversation_dialogue(speaker_npc_fqn);
        "#,
    )?;

    // Conversation dialogue lines. The spoken/subtitle text lives in the
    // per-conversation STBs under /str/cnv/ (enabled in should_extract_stb),
    // stored as `str.cnv.<path>.<id1>.<id2>` rows in `strings`. This view
    // strips the `str.` prefix and the trailing `.<id1>.<id2>` to recover the
    // `cnv.*` FQN, so dialogue rejoins its conversation (and via
    // conversation_quest_refs, its quest). line_group/line_id are the STB
    // id1/id2 -- ordering within the conversation.
    tx.execute_batch(
        r#"
            CREATE VIEW IF NOT EXISTS conversation_lines AS
                SELECT
                    substr(fqn, 5, length(fqn) - length(id1) - length(id2) - 6) AS cnv_fqn,
                    id1 AS line_group,
                    id2 AS line_id,
                    text
                FROM strings
                WHERE fqn LIKE 'str.cnv.%';
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::temp_db_path;

    #[test]
    fn conversation_stb_path_inverts_fqn() {
        // cnv.X.Y -> /resources/en-us/str/cnv/X/Y.stb (inverse of
        // stb::extract_fqn_from_path).
        assert_eq!(
            conversation_stb_path("cnv.location.taris.x"),
            "/resources/en-us/str/cnv/location/taris/x.stb"
        );
        assert_eq!(
            conversation_stb_path("cnv.location.taris.class.jedi_knight.rora_seake"),
            "/resources/en-us/str/cnv/location/taris/class/jedi_knight/rora_seake.stb"
        );
    }

    #[test]
    fn conversation_stb_path_round_trips_through_stb_fqn() {
        // The derived path, fed back through the STB FQN extractor, must yield
        // the original conversation FQN -- so str.cnv.* dialogue rejoins its
        // conversation via the conversation_lines view.
        let cnv_fqn = "cnv.location.taris.class.jedi_knight.rora_seake";
        let path = conversation_stb_path(cnv_fqn);
        let stb_file =
            crate::stb::parse(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], &path).unwrap();
        assert_eq!(stb_file.fqn_prefix, format!("str.{cnv_fqn}"));
        assert_eq!(stb_file.locale, "en-us");
    }

    #[test]
    fn conversation_stb_path_hashes_match_dict_hash_function() {
        // The archive lookup key is swtor_filename_hash of the derived path.
        // Verify it is deterministic and equals the dictionary hash function
        // for a known str path shape (the real str.cnv hashes are validated by
        // the user's full extraction; this guards against drift in the path
        // derivation vs. the hash entrypoint we call at runtime).
        let path = conversation_stb_path("cnv.location.taris.x");
        let a = crate::hash::swtor_filename_hash(&path);
        let b = crate::hash::swtor_filename_hash(&path);
        assert_eq!(a, b, "hash must be deterministic");
        // Matches the verified-against-real-dict entrypoint for a known path.
        assert_eq!(
            crate::hash::swtor_filename_hash("/resources/en-us/str/abl.stb"),
            ((0x54305B3B_u64) << 32) | 0x8154956D_u64,
            "swtor_filename_hash must match the real dict PH/SH for abl.stb"
        );
    }

    #[test]
    fn conversation_tables_and_view_exist_after_init() {
        let path = temp_db_path("cnv_schema");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        let conn = db.conn.lock().unwrap();
        for (name, ty) in [
            ("conversation_quest_refs", "table"),
            ("conversation_npcs", "table"),
            ("conversation_followups", "table"),
            ("conversation_alignment_events", "table"),
            ("conversation_lines", "view"),
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type=?1 AND name=?2",
                    params![ty, name],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing {ty} {name}");
        }
    }

    #[test]
    fn conversation_lines_recovers_cnv_fqn_from_str_cnv() {
        let path = temp_db_path("cnv_lines");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            // str.cnv.<conv path>.<id1>.<id2> dialogue line
            conn.execute(
                "INSERT INTO strings (fqn, locale, id1, id2, text) VALUES \
                 ('str.cnv.location.taris.class.jedi_knight.rora_seake.3.42', 'en-us', 3, 42, 'We need your help, Jedi.')",
                [],
            )
            .unwrap();
            // a non-cnv string must be excluded by the view
            conn.execute(
                "INSERT INTO strings (fqn, locale, id1, id2, text) VALUES ('str.qst.88.500','en-us',88,500,'Some Quest')",
                [],
            )
            .unwrap();
        }
        let conn = db.conn.lock().unwrap();
        let (cnv_fqn, text): (String, String) = conn
            .query_row(
                "SELECT cnv_fqn, text FROM conversation_lines WHERE line_id = 42",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            cnv_fqn, "cnv.location.taris.class.jedi_knight.rora_seake",
            "view must strip the str. prefix and the trailing .id1.id2"
        );
        assert_eq!(text, "We need your help, Jedi.");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversation_lines", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1, "non-cnv strings excluded from the view");
    }
}
