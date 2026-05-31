//! For every quest, run the walker and emit `(hi32, kind, value_or_type)`
//! triples. Aggregate by hi32 to see which hashes carry stable enum-like
//! data (small set of distinct values across the corpus) vs metadata
//! artifacts (constant value, typically -50 from 0xCE).
//!
//! Usage: quest_hash_stats <spice.sqlite>

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use kessel::schema::decode_payload_schema_aware;
use rusqlite::Connection;
use std::collections::HashMap;

const QUEST_HI32: u32 = 0x2ADEC3D2;

fn main() -> Result<()> {
    let db_path = std::env::args().nth(1).context("db path required")?;
    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT json_extract(json, '$.payload_b64') \
         FROM objects WHERE kind='Quest' AND is_canonical=1",
    )?;
    let payloads: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    // hi32 -> (kind, value->count)
    let mut per_hash: HashMap<String, (String, HashMap<String, u32>)> = HashMap::new();

    for b64 in &payloads {
        let Ok(payload) = B64.decode(b64) else {
            continue;
        };
        let Ok(decoded) = decode_payload_schema_aware(&payload, QUEST_HI32) else {
            continue;
        };
        let Some(named) = decoded.named_props.as_object() else {
            continue;
        };
        for (k, v) in named {
            let parts: Vec<&str> = k.split("__").collect();
            if parts.len() < 2 {
                continue;
            }
            let kind = parts[0].to_string();
            let hi32 = parts[1].to_string();
            let val_repr = if let Some(s) = v.as_str() {
                format!("STR:{s}")
            } else if let Some(n) = v.as_i64() {
                format!("INT:{n}")
            } else if let Some(b) = v.as_bool() {
                format!("BOOL:{b}")
            } else {
                continue;
            };
            let entry = per_hash.entry(hi32).or_insert((kind, HashMap::new()));
            *entry.1.entry(val_repr).or_insert(0) += 1;
        }
    }

    // Sort by total count descending.
    let mut rows: Vec<(String, String, HashMap<String, u32>)> = per_hash
        .into_iter()
        .map(|(hi32, (kind, vals))| (hi32, kind, vals))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2.values().sum::<u32>()));

    println!("hi32\tkind\tdistinct\ttotal\ttop_5_values");
    for (hi32, kind, vals) in rows {
        let total: u32 = vals.values().sum();
        let n_distinct = vals.len();
        let mut sorted: Vec<(String, u32)> = vals.into_iter().collect();
        sorted.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        let top: Vec<String> = sorted
            .iter()
            .take(5)
            .map(|(v, c)| {
                let trimmed: String = v.chars().take(40).collect();
                format!("{trimmed}({c})")
            })
            .collect();
        println!("{hi32}\t{kind}\t{n_distinct}\t{total}\t{}", top.join(", "));
    }
    Ok(())
}
