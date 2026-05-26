//! Same as probe_typed_encoding but captures up to 256 bytes after each marker
//! so variable-length encodings (arrays, wrappers) can be characterized.
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
        .unwrap_or("/tmp/typed-encoding-fulltails.jsonl".into());
    let target_tag = std::env::args().nth(3); // optional: only emit for one tag
    let target_byte = target_tag
        .as_ref()
        .and_then(|s| u8::from_str_radix(s, 16).ok());

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
    eprintln!(
        "scanning {} canonical objects, target_tag={:?}",
        rows.len(),
        target_byte
    );

    let mut out = BufWriter::new(File::create(&out_path)?);
    let mut total = 0u64;
    for (kind, fqn, b64) in &rows {
        let Some(b64) = b64 else { continue };
        let Ok(p) = B64.decode(b64) else { continue };
        let mut i = 0;
        while i + 9 <= p.len() {
            if !(p[i] == 0xCF && p[i + 1] == 0x40 && p[i + 2] == 0x00 && p[i + 3] == 0x00) {
                i += 1;
                continue;
            }
            let tail_start = i + 9;
            if tail_start >= p.len() {
                break;
            }
            if let Some(tb) = target_byte {
                if p[tail_start] != tb {
                    i += 9;
                    continue;
                }
            }
            let tail_end = (tail_start + 256).min(p.len());
            let hi32 = u32::from_be_bytes(p[i + 5..i + 9].try_into().unwrap());
            let hi32_hex = format!("{hi32:08X}");
            let tail_hex = hex::encode_upper(&p[tail_start..tail_end]);
            let schema = gom_schema::property_for_cf40(hi32)
                .map(|p| (p.kind.clone(), p.type_tag))
                .unwrap_or((String::from("unresolved"), 0));
            writeln!(
                out,
                "{{\"k\":\"{}\",\"f\":\"{}\",\"o\":{},\"h\":\"{}\",\"sk\":\"{}\",\"st\":{},\"t\":\"{}\"}}",
                kind, fqn, i, hi32_hex, schema.0, schema.1, tail_hex
            )?;
            total += 1;
            i += 9;
        }
    }
    out.flush()?;
    eprintln!("wrote {} markers to {}", total, out_path);
    Ok(())
}
