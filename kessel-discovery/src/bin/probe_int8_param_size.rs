//! Measure the byte size between an int8/int16 param-array opener
//! `[01|07] 08 05 <02|03> <count_a> <count_b>` and the next property
//! opener. Divide by count_a to infer item size empirically.
//!
//! Usage: probe_int8_param_size <spice.sqlite>

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::Connection;
use std::collections::HashMap;
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let db_path = args.get(1).context("usage: probe_int8_param_size <db>")?;

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'abl.%'",
    )?;
    let payloads: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Map (val_tag, count_a) -> [size_per_item_observed]
    let mut buckets: HashMap<(u8, u8), Vec<usize>> = HashMap::new();
    // Track count_a vs count_b mismatch frequency.
    let mut count_mismatch: HashMap<u8, (u32, u32)> = HashMap::new(); // val_tag -> (match, mismatch)

    for b64 in &payloads {
        let Ok(payload) = BASE64.decode(b64) else {
            continue;
        };

        let mut i = 0;
        while i + 6 <= payload.len() {
            let wrapper_ok = payload[i] == 0x01 || payload[i] == 0x07;
            let opener_ok = wrapper_ok && payload[i + 1] == 0x08 && payload[i + 2] == 0x05;
            if !opener_ok {
                i += 1;
                continue;
            }
            let val_tag = payload[i + 3];
            if !matches!(val_tag, 0x02 | 0x03) {
                i += 1;
                continue;
            }
            let count_a = payload[i + 4];
            let count_b = payload[i + 5];
            let entry = count_mismatch.entry(val_tag).or_insert((0, 0));
            if count_a == count_b {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }

            if count_a == 0 || count_a > 64 {
                i += 1;
                continue;
            }
            // Find next property opener `[01|07] 08 05` after items_start.
            let items_start = i + 6;
            let mut j = items_start + 1;
            let mut next_opener = None;
            while j + 3 <= payload.len() {
                let w = payload[j] == 0x01 || payload[j] == 0x07;
                if w && payload[j + 1] == 0x08 && payload[j + 2] == 0x05 {
                    next_opener = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(next) = next_opener {
                let total_bytes = next - items_start;
                if count_a > 0 && total_bytes % count_a as usize == 0 {
                    let item_size = total_bytes / count_a as usize;
                    if item_size <= 32 {
                        buckets
                            .entry((val_tag, count_a))
                            .or_default()
                            .push(item_size);
                    }
                }
            }
            i = items_start;
        }
    }

    // Report: for each (val_tag, count_a), what item_size(s) dominate?
    let mut keys: Vec<(u8, u8)> = buckets.keys().copied().collect();
    keys.sort();
    println!("val_tag\tcount_a\tn_samples\tmedian_item_size\tmin\tmax\tunique_sizes");
    for k in &keys {
        let v = &buckets[k];
        if v.len() < 5 {
            continue;
        }
        let mut sorted = v.clone();
        sorted.sort();
        let median = sorted[sorted.len() / 2];
        let min = *sorted.first().unwrap();
        let max = *sorted.last().unwrap();
        let mut uniq: Vec<usize> = sorted.iter().copied().collect();
        uniq.dedup();
        let uniq_str: Vec<String> = uniq.iter().map(|n| n.to_string()).collect();
        println!(
            "0x{:02X}\t{}\t{}\t{}\t{}\t{}\t{}",
            k.0,
            k.1,
            v.len(),
            median,
            min,
            max,
            uniq_str.join(",")
        );
    }
    println!();
    println!("--- count_a == count_b match rate ---");
    for (vt, (m, mm)) in &count_mismatch {
        println!("val_tag=0x{vt:02X}: match={m} mismatch={mm}");
    }

    Ok(())
}
