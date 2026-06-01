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
    /// Populate the conversation reference graph by scanning every NODE
    /// prototype file in `tor_dir` for CF GUID refs and alignment-event tokens.
    ///
    /// NODE files at `/resources/systemgenerated/prototypes/<num>.node` hold
    /// the playback data for `cnv.*` objects. The PROT header (bytes 0x14..)
    /// carries the cnv FQN. The body contains CF E0 GUID refs; each is resolved
    /// against the object GUID map and bucketed by the target's kind
    /// (Quest/Npc/Achievement/Codex/Item/Conversation/Encounter). Alignment
    /// beats are counted from `event.darkmoment` / `heroicmoment` / ... tokens.
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
