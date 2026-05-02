//! Search every object payload + header for byte-level references to ANY of
//! the 8 GSF companion NPC GUIDs. Mirrors probe_evasion_byteref but flipped:
//! "does anything point at a spvp companion?" rather than "does anything
//! point at a crew talent?"

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: probe_companion_byteref <spice.sqlite>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let conn = Connection::open(&path)?;

    // 1. Load companion guids.
    let mut companions: HashMap<[u8; 8], String> = HashMap::new();
    let mut stmt =
        conn.prepare("SELECT fqn, guid FROM objects WHERE fqn LIKE 'npc.companion.spvp.%'")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows.filter_map(|r| r.ok()) {
        let (fqn, guid_hex) = row;
        let guid = u64::from_str_radix(&guid_hex, 16)?;
        companions.insert(guid.to_le_bytes(), fqn);
    }
    println!("companion GUIDs loaded: {}", companions.len());
    for (b, f) in &companions {
        println!("  {} (LE: {:02X?})", f, b);
    }
    println!();

    // 2. Scan all objects.
    let mut stmt = conn.prepare(
        "SELECT fqn, \
                json_extract(json, '$.payload_b64'), \
                json_extract(json, '$.header_hex') \
         FROM objects",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut hits: Vec<(String, String, &'static str)> = Vec::new();
    let mut total = 0usize;
    for row in rows.filter_map(|r| r.ok()) {
        total += 1;
        let (fqn, payload_b64, header_hex) = row;
        let payload = BASE64.decode(payload_b64.as_bytes()).unwrap_or_default();
        let header = hex::decode(&header_hex).unwrap_or_default();

        // Skip the companion's own row in payload+header.
        if fqn.starts_with("npc.companion.spvp.") {
            continue;
        }

        for win_size in [8usize] {
            for buf in [(&payload[..], "payload"), (&header[..], "header")] {
                let (bytes, source) = buf;
                if bytes.len() < win_size {
                    continue;
                }
                for w in bytes.windows(win_size) {
                    let arr: [u8; 8] = w.try_into().unwrap();
                    if let Some(comp_fqn) = companions.get(&arr) {
                        hits.push((fqn.clone(), comp_fqn.clone(), source));
                    }
                }
            }
        }
    }

    println!(
        "scanned {} objects (excluding the 8 companions themselves)",
        total
    );
    println!("\n=== hits ===");
    if hits.is_empty() {
        println!("(none)");
    } else {
        for (owner, target, source) in &hits {
            println!("  {} ({})  ->  {}", owner, source, target);
        }
    }

    Ok(())
}
