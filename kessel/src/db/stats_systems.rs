//! Armor-class taxonomy and per-level stat curve extraction.

use super::*;

impl Database {
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
}

/// Create the stats_systems tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
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
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
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
}
