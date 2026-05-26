use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use kessel::schema::decode_payload_schema_aware;
use rusqlite::Connection;

fn main() -> anyhow::Result<()> {
    const QUEST_HI32: u32 = 0x2ADEC3D2;
    let conn = Connection::open("/tmp/spice-wired.sqlite")?;
    let mut stmt = conn.prepare("SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind='Quest' AND is_canonical=1 LIMIT 3")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (fqn, b64) = row?;
        let payload = B64.decode(&b64)?;
        let decoded = decode_payload_schema_aware(&payload, QUEST_HI32)?;
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
        // Just print the keys + first-20-char value
        if let Some(m) = decoded.named_props.as_object() {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys.iter().take(20) {
                let v = &m[*k];
                let vs = v.to_string();
                let vs = if vs.len() > 80 {
                    format!("{}...", &vs[..80])
                } else {
                    vs
                };
                println!("  {}: {}", k, vs);
            }
            if keys.len() > 20 {
                println!("  ... ({} more keys)", keys.len() - 20);
            }
        }
    }
    Ok(())
}
