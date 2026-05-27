//! Dump the schema-aware walker's named_props output for a sample of
//! objects in spice.sqlite. Useful for spotting which property hashes
//! actually carry data and what shapes they decode to.
//!
//! Usage:
//!   probe_walker_output [db_path] [limit] [class_hi32_hex] [kind_filter]
//!
//! Defaults: spice-actparams.sqlite, 3 objects, Quest class.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use kessel::schema::decode_payload_schema_aware;
use rusqlite::Connection;

fn main() -> anyhow::Result<()> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/spice-actparams.sqlite".to_string());
    let limit: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let class_hi32 = std::env::args()
        .nth(3)
        .and_then(|s| u32::from_str_radix(&s, 16).ok())
        .unwrap_or(0x2ADEC3D2);
    let kind_filter = std::env::args()
        .nth(4)
        .unwrap_or_else(|| "Quest".to_string());

    let conn = Connection::open(&db_path)?;
    let sql = format!(
        "SELECT fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE kind=?1 AND is_canonical=1 LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    for row in stmt.query_map([&kind_filter], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (fqn, b64) = row?;
        let payload = B64.decode(&b64)?;
        let decoded = decode_payload_schema_aware(&payload, class_hi32)?;
        println!(
            "\n=== {} ({} bytes) raw={} named={} resolved={} unresolved={} ===",
            fqn,
            payload.len(),
            decoded.raw_props.as_object().map(|m| m.len()).unwrap_or(0),
            decoded
                .named_props
                .as_object()
                .map(|m| m.len())
                .unwrap_or(0),
            decoded.property_count_resolved,
            decoded.property_count_unresolved
        );
        if let Some(m) = decoded.named_props.as_object() {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys.iter().take(80) {
                let v = &m[*k];
                let vs = v.to_string();
                let vs = if vs.len() > 120 {
                    format!("{}...", &vs[..120])
                } else {
                    vs
                };
                println!("  {}: {}", k, vs);
            }
            if keys.len() > 80 {
                println!("  ... ({} more keys)", keys.len() - 80);
            }
        }
    }
    Ok(())
}
