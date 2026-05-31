//! Shared test harness (temp DB path + singleton seeding) for db domain tests.

use crate::db::*;
use rusqlite::params;

pub(crate) fn temp_db_path(label: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("kessel_test_{}_{}_{}.sqlite", label, pid, nanos))
}

/// Seed a `singletons` row with a raw payload (base64-encoded) for the
/// per-singleton decoder tests.
pub(crate) fn seed_singleton(db: &Database, fqn: &str, payload: &[u8]) {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO singletons \
            (fqn, payload_size, payload_b64, string_count, cf_e0_count, cf_40_count, header_hex) \
         VALUES (?1, ?2, ?3, 0, 0, 0, '')",
        params![fqn, payload.len() as i64, BASE64.encode(payload)],
    )
    .unwrap();
}

/// Helper: insert an Ability or Talent object so populate_disciplines and
/// populate_discipline_talents have something to find.
pub(crate) fn insert_obj(db: &Database, game_id: &str, fqn: &str, kind: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
         VALUES (?1, 'sid', 'ph', 'guid', ?2, ?3, '{}')",
        params![game_id, fqn, kind],
    )
    .unwrap();
}

/// Helper: seed `combat_styles` rows for a given origin so disciplines/css/cut
/// inserts can satisfy their FK to combat_styles(fqn_segment).
pub(crate) fn seed_combat_styles_for(db: &Database, origin: &str) {
    let conn = db.conn.lock().unwrap();
    for cs in origin_combat_styles(origin) {
        let game_id = format!("cs_{}", cs);
        let fqn = format!("class.pc.advanced.{}", cs);
        conn.execute(
            "INSERT OR IGNORE INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'class', '{}')",
            params![game_id, format!("sid_{}", cs), format!("ph_{}", cs), format!("guid_{}", cs), fqn],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO combat_styles \
               (combat_style_id, fqn, fqn_segment, display_segment, faction, attack_type) \
             VALUES (?1, ?2, ?3, ?3, 'unknown', 'unknown')",
            params![game_id, fqn, cs],
        )
        .unwrap();
    }
}
