//! Shared test harness (temp DB path + singleton seeding) for db domain tests.

use crate::db::Database;
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
