//! Search every object payload for byte-level references to
//! `tal.spvp.crew.defensive.evasion` (the +5% Evasion crew talent):
//!   - GUID  E000263B65AEA214 as LE u64 in payload
//!   - GUID  E000263B65AEA214 as LE u64 in header
//!   - string_id 770237 (0xBC0BD) as LE u32 in payload
//!   - same as LE u16 (in case it's a smaller field)
//!
//! Reports any object whose bytes match -- which would be a candidate "owner"
//! that grants this crew talent.

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::Connection;
use std::path::PathBuf;

const TARGET_GUID_HEX: &str = "E000263B65AEA214";
const TARGET_STRING_ID: u32 = 770237;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: probe_evasion_byteref <spice.sqlite>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let conn = Connection::open(&path)?;

    let target_guid = u64::from_str_radix(TARGET_GUID_HEX, 16)?;
    let guid_le_bytes = target_guid.to_le_bytes();
    let guid_be_bytes = target_guid.to_be_bytes();
    let sid_le4 = TARGET_STRING_ID.to_le_bytes();
    let sid_le2 = (TARGET_STRING_ID as u16).to_le_bytes();

    println!("target talent: tal.spvp.crew.defensive.evasion");
    println!(
        "  guid: {} (LE bytes: {:02X?}, BE bytes: {:02X?})",
        TARGET_GUID_HEX, guid_le_bytes, guid_be_bytes
    );
    println!(
        "  string_id: {} (LE u32 bytes: {:02X?})",
        TARGET_STRING_ID, sid_le4
    );
    println!();

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

    let mut total = 0usize;
    let mut hit_guid_payload_le = Vec::new();
    let mut hit_guid_payload_be = Vec::new();
    let mut hit_guid_header_le = Vec::new();
    let mut hit_sid_payload_u32 = Vec::new();
    let mut hit_sid_payload_u16 = Vec::new();

    for row in rows.filter_map(|r| r.ok()) {
        total += 1;
        let (fqn, payload_b64, header_hex) = row;
        let payload = match BASE64.decode(payload_b64.as_bytes()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let header = hex::decode(&header_hex).unwrap_or_default();

        if find_window(&payload, &guid_le_bytes) {
            hit_guid_payload_le.push(fqn.clone());
        }
        if find_window(&payload, &guid_be_bytes) {
            hit_guid_payload_be.push(fqn.clone());
        }
        if find_window(&header, &guid_le_bytes) {
            hit_guid_header_le.push(fqn.clone());
        }
        if find_window(&payload, &sid_le4) {
            hit_sid_payload_u32.push(fqn.clone());
        }
        if find_window(&payload, &sid_le2) {
            hit_sid_payload_u16.push(fqn.clone());
        }
    }

    println!("scanned {} objects\n", total);

    fn report(label: &str, hits: &[String]) {
        println!("=== {} -- {} hit(s) ===", label, hits.len());
        for h in hits.iter().take(60) {
            println!("  {}", h);
        }
        if hits.len() > 60 {
            println!("  ... ({} more)", hits.len() - 60);
        }
        println!();
    }

    report("GUID match in payload (LE)", &hit_guid_payload_le);
    report("GUID match in payload (BE)", &hit_guid_payload_be);
    report("GUID match in header (LE)", &hit_guid_header_le);
    report("string_id match in payload (LE u32)", &hit_sid_payload_u32);
    report("string_id match in payload (LE u16)", &hit_sid_payload_u16);

    Ok(())
}

fn find_window(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}
