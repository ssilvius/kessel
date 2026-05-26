//! Investigation probe for the typed-value encoding model.
//!
//! For every canonical PBUK object in spice, walk every CF40 marker.
//! For each marker, record:
//!   - object kind (Quest/Ability/Item/Npc/Talent/...)
//!   - type_flags byte (the byte at position +4 after CF 40 00 00)
//!   - property hi32 (4 bytes BE at +5..+9)
//!   - schema type_tag + kind looked up via property_for_cf40
//!   - first 32 bytes of value tail (the bytes after the 9-byte marker)
//!
//! Outputs JSON-lines to /tmp/typed-encoding-samples.jsonl so subsequent
//! Python/SQL analysis can bucket and pattern-match.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use kessel::gom_schema;
use rusqlite::Connection;
use std::fs::File;
use std::io::{BufWriter, Write};

fn main() -> Result<()> {
    let db = std::env::args()
        .nth(1)
        .unwrap_or("/tmp/spice-178.sqlite".into());
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or("/tmp/typed-encoding-samples.jsonl".into());
    let conn = Connection::open(&db)?;

    let mut stmt = conn.prepare(
        "SELECT kind, fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE is_canonical = 1",
    )?;
    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    eprintln!("scanning {} canonical objects", rows.len());

    let mut out = BufWriter::new(File::create(&out_path)?);
    let mut total_markers = 0u64;
    let mut total_unresolved = 0u64;

    for (kind, fqn, b64) in &rows {
        let Some(b64) = b64 else { continue };
        let Ok(p) = B64.decode(b64) else { continue };

        let mut i = 0;
        while i + 9 <= p.len() {
            if !(p[i] == 0xCF && p[i + 1] == 0x40 && p[i + 2] == 0x00 && p[i + 3] == 0x00) {
                i += 1;
                continue;
            }
            let type_flags = p[i + 4];
            let hi32_bytes: [u8; 4] = p[i + 5..i + 9].try_into().unwrap();
            let hi32 = u32::from_be_bytes(hi32_bytes);
            let hi32_hex = format!("{hi32:08X}");

            let tail_start = i + 9;
            let tail_end = (tail_start + 32).min(p.len());
            let tail_hex = hex::encode_upper(&p[tail_start..tail_end]);

            let (schema_kind, schema_type_tag, schema_refs) =
                match gom_schema::property_for_cf40(hi32) {
                    Some(prop) => {
                        let refs = prop
                            .refs
                            .as_ref()
                            .map(|r| {
                                r.iter()
                                    .map(|x| format!("{}:{}", x.kind, x.name))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_default();
                        (prop.kind.clone(), prop.type_tag, refs)
                    }
                    None => {
                        total_unresolved += 1;
                        (String::from("unresolved"), 0u8, String::new())
                    }
                };

            // JSONL: one record per CF40 marker
            writeln!(
                out,
                "{{\"kind\":\"{}\",\"fqn\":\"{}\",\"off\":{},\"type_flags\":\"{:02X}\",\"hi32\":\"{}\",\"schema_kind\":\"{}\",\"schema_type_tag\":\"{:02X}\",\"schema_refs\":\"{}\",\"tail32\":\"{}\"}}",
                kind, fqn, i, type_flags, hi32_hex, schema_kind, schema_type_tag, schema_refs, tail_hex
            )?;
            total_markers += 1;
            i += 9;
        }
    }
    out.flush()?;
    eprintln!(
        "wrote {} markers ({} unresolved) to {}",
        total_markers, total_unresolved, out_path
    );
    Ok(())
}
