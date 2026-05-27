//! Dump real byte samples for specific (val_tag, count_a) int8 param
//! shapes so we can eyeball the item layout.
//!
//! Usage: probe_int8_samples <spice.sqlite> <val_tag_hex> <count_a> [n_samples=20] [tail_bytes=24]

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::Connection;
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let db_path = args.get(1).context("db path required")?;
    let want_tag = u8::from_str_radix(args.get(2).context("val_tag hex required")?, 16)?;
    let want_count: u8 = args.get(3).context("count_a required")?.parse()?;
    let n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
    let tail_bytes: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(24);

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'abl.%'",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut emitted = 0;
    'outer: for (fqn, b64) in &rows {
        let Ok(payload) = BASE64.decode(b64) else {
            continue;
        };
        let mut i = 0;
        while i + 6 <= payload.len() {
            let wrapper_ok = payload[i] == 0x01 || payload[i] == 0x07;
            if !wrapper_ok
                || payload[i + 1] != 0x08
                || payload[i + 2] != 0x05
                || payload[i + 3] != want_tag
                || payload[i + 4] != want_count
                || payload[i + 5] != want_count
            {
                i += 1;
                continue;
            }
            let start = i + 6;
            let end = (start + tail_bytes).min(payload.len());
            println!("{}\t{}", fqn, hex::encode(&payload[start..end]));
            emitted += 1;
            if emitted >= n {
                break 'outer;
            }
            i = start;
        }
    }
    Ok(())
}
