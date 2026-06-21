//! Corpus description-anchoring pass (#324, epic #322): parse every canonical
//! object's `id1=1` description and write the typed numeric facts to
//! `description_values`, keyed by object. Domain-agnostic landing surface that
//! the per-domain promotions (#325/#326) read from.

use super::*;
use crate::schema::description_anchor::{parse_description, FactKind};

impl Database {
    /// Parse id1=1 descriptions for canonical named objects and write their
    /// `AnchoredFact`s to `description_values`. Returns (objects_with_facts,
    /// total_facts).
    pub fn populate_description_values(&self) -> Result<(u64, u64)> {
        // (game_id, fqn, string_id, description text). en-us, id1=1, prefer the
        // raw pre-grammar text so `<<N>>` templates survive.
        let conn = self.conn.lock().unwrap();

        let rows: Vec<(String, String, i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT o.game_id, o.fqn, o.string_id, COALESCE(s.text_raw, s.text) \
                 FROM objects o \
                 JOIN strings s ON s.id2 = o.string_id AND s.id1 = 1 AND s.locale = 'en-us' \
                 WHERE o.is_canonical = 1 AND o.string_id IS NOT NULL",
            )?;
            let collected: Vec<(String, String, i64, String)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO description_values \
             (object_game_id, fqn, string_id, seq, kind, value, label, token_ordinal) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;

        let (mut objects, mut facts) = (0u64, 0u64);
        for (game_id, fqn, string_id, text) in &rows {
            let mut parsed = parse_description(text);
            if parsed.is_empty() {
                continue;
            }
            // parse_description emits stat-phrase matches before number-unit
            // matches; sort by source offset so `seq` is true reading order.
            parsed.sort_by_key(|f| f.at);
            objects += 1;
            for (seq, fact) in parsed.iter().enumerate() {
                let (kind, value, token_ordinal): (&str, Option<f64>, Option<i64>) = match fact.kind
                {
                    FactKind::Percent => ("percent", Some(fact.value), None),
                    FactKind::DurationSeconds => ("duration_seconds", Some(fact.value), None),
                    FactKind::RangeMeters => ("range_meters", Some(fact.value), None),
                    FactKind::Count => ("count", Some(fact.value), None),
                    FactKind::Magnitude => ("magnitude", Some(fact.value), None),
                    FactKind::Template(n) => ("template", None, Some(i64::from(n))),
                };
                stmt.execute(params![
                    game_id,
                    fqn,
                    string_id,
                    seq as i64,
                    kind,
                    value,
                    fact.label,
                    token_ordinal,
                ])?;
                facts += 1;
            }
        }

        drop(stmt);
        tx.commit()?;
        Ok((objects, facts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::temp_db_path;

    #[test]
    fn writes_expected_facts_for_known_object() {
        let path = temp_db_path("description_values");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, is_canonical, json) \
                 VALUES ('g1', 'sid', 'ph', 'guid', 'abl.test.x', 'Ability', 555, 1, '{}')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO strings (fqn, locale, id1, id2, text, text_raw) VALUES \
                 ('str.abl.test.x', 'en-us', 1, 555, \
                  'grant 510 Power for seconds. once every 20 seconds.', \
                  'grant 510 Power for <<1>> seconds. This effect can only occur once every 20 seconds.')",
                [],
            )
            .unwrap();
        }

        let (objects, facts) = db.populate_description_values().unwrap();
        assert_eq!(objects, 1);
        assert!(facts >= 3);

        let conn = db.conn.lock().unwrap();
        let mag: f64 = conn
            .query_row(
                "SELECT value FROM description_values WHERE object_game_id='g1' AND kind='magnitude' AND label='power'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mag, 510.0);
        let icd: f64 = conn
            .query_row(
                "SELECT value FROM description_values WHERE object_game_id='g1' AND kind='duration_seconds' AND value=20.0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(icd, 20.0);
        let ord: i64 = conn
            .query_row(
                "SELECT token_ordinal FROM description_values WHERE object_game_id='g1' AND kind='template'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ord, 1);
    }
}
