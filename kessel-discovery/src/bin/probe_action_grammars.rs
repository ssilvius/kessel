//! Probe per-effAction byte shapes after the action enum byte.
//!
//! Reads abl.* objects from a spice.sqlite, finds each effAction marker
//! (CF 40 00 00 ?? E2 51 D1 CC 05 <enum_idx>), and records the next N
//! bytes up to the next CF40/CB/CC/CD/CE layer marker. Groups samples by
//! effAction enum_member and writes JSONL to stdout (one record per
//! sample) for downstream pattern mining.
//!
//! Usage:
//!   probe_action_grammars <spice.sqlite> [tail_max_bytes=64] [samples_per_action=20]

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_schema;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::env;

#[derive(Serialize)]
struct Sample<'a> {
    action: &'a str,
    action_idx: u8,
    fqn: &'a str,
    effect_ord: u32,
    tail_hex: String,
    tail_len: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let db_path = args
        .get(1)
        .context("usage: probe_action_grammars <spice.sqlite> [tail_max] [samples_per]")?;
    let tail_max: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(64);
    let samples_per: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);

    let eff_action_enum =
        gom_schema::enum_for_name("effAction").context("effAction enum missing from gom_schema")?;

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'abl.%'",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut counts: HashMap<u8, usize> = HashMap::new();

    for (fqn, b64) in &rows {
        let Ok(payload) = BASE64.decode(b64) else {
            continue;
        };
        let mut effect_ord: u32 = 0;
        let mut i = 0;
        while i + 11 <= payload.len() {
            let is_marker = payload[i] == 0xCF
                && payload[i + 1] == 0x40
                && payload[i + 2] == 0x00
                && payload[i + 3] == 0x00
                && payload[i + 5..i + 9] == [0xE2, 0x51, 0xD1, 0xCC]
                && payload[i + 9] == 0x05;
            if !is_marker {
                i += 1;
                continue;
            }
            let action_idx = payload[i + 10];
            let action_name = eff_action_enum
                .members
                .get(action_idx as usize)
                .map(String::as_str)
                .unwrap_or("?");

            let entry = counts.entry(action_idx).or_insert(0);
            if *entry < samples_per {
                let tail_start = i + 11;
                let mut tail_end = (tail_start + tail_max).min(payload.len());
                // Cut off at next layer marker (CB CC CD CE CF) — these
                // open new typed-property or metadata records.
                let mut k = tail_start;
                while k < tail_end {
                    let b = payload[k];
                    if matches!(b, 0xCB | 0xCC | 0xCD | 0xCE | 0xCF) && k > tail_start + 1 {
                        tail_end = k;
                        break;
                    }
                    k += 1;
                }
                let tail = &payload[tail_start..tail_end];
                let s = Sample {
                    action: action_name,
                    action_idx,
                    fqn,
                    effect_ord,
                    tail_hex: hex::encode(tail),
                    tail_len: tail.len(),
                };
                println!("{}", serde_json::to_string(&s)?);
                *entry += 1;
            }
            effect_ord += 1;
            i += 11;
        }
    }

    Ok(())
}
