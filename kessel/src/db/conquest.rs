//! Conquest objectives, events, and weekly schedule extraction.

use super::*;

impl Database {
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
}

/// Parse a conquest objective FQN (`ach.conquests.<category>.<sub>...<leaf>`)
/// into (category, subcategory, cadence). Cadence is `Some("weekly")` if the
/// leaf ends with `_weekly` or path contains `.weekly.`, `Some("daily")` if
/// the path contains `.daily.`, otherwise `None` for repeatable objectives.
pub(crate) fn parse_conquest_fqn(fqn: &str) -> (String, Option<String>, Option<String>) {
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

/// Create the conquest tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
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
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
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
}
