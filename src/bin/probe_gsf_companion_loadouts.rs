//! Probe whether `npc.companion.spvp.*` payloads embed GUID references to the
//! crew talent (`tal.spvp.crew.*`) and crew ability (`abl.spvp.crew.*`)
//! objects that define each GSF companion's two passive stat modifiers and
//! single active ability.
//!
//! Strategy: build a GUID -> fqn map for every `tal.spvp.crew.*` and
//! `abl.spvp.crew.*` object, then for each `npc.companion.spvp.*` decode
//! its payload and (a) pull every `cf XX..` GUID-marker record and (b)
//! brute-force every overlapping 8-byte window against the crew GUID set
//! to catch references that don't use the `cf` marker. Report matches
//! per companion plus a summary.
//!
//! Usage: ./target/release/probe_gsf_companion_loadouts <spice.sqlite>

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: probe_gsf_companion_loadouts <spice.sqlite>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let conn = Connection::open(&path)?;

    // 1. Build crew GUID -> (kind_tag, fqn) lookup.
    //    kind_tag is "tal" or "abl" so we can classify matches.
    let mut crew: HashMap<String, (&'static str, String)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT guid, fqn FROM objects \
             WHERE fqn LIKE 'tal.spvp.crew.%' OR fqn LIKE 'abl.spvp.crew.%'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows.filter_map(|r| r.ok()) {
            let (guid, fqn) = row;
            let kind = if fqn.starts_with("tal.") {
                "tal"
            } else {
                "abl"
            };
            crew.insert(guid.to_uppercase(), (kind, fqn));
        }
    }
    println!("crew object GUIDs loaded: {}", crew.len());

    // 2. Walk every npc.companion.spvp.* payload AND header.
    //    Header layout known: bytes 0-7 = content GUID, 16-23 = template GUID.
    //    Bytes 8-15 and 24-41 are unparsed -- prime candidates for embedded
    //    GUID refs (e.g. crew talents/abilities).
    let mut stmt = conn.prepare(
        "SELECT fqn, \
                json_extract(json, '$.payload_b64'), \
                json_extract(json, '$.header_hex') \
         FROM objects WHERE fqn LIKE 'npc.companion.spvp.%' \
         ORDER BY fqn",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut summary_cf_hits: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    let mut summary_window_hits: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    let mut crew_object_hit_count: HashMap<String, usize> = HashMap::new();

    type HeaderHit = (Vec<String>, Vec<String>, Vec<(usize, String)>);
    let mut summary_header_hits: BTreeMap<String, HeaderHit> = BTreeMap::new();

    for row in rows.filter_map(|r| r.ok()) {
        let (fqn, payload_b64, header_hex) = row;
        let payload = BASE64.decode(payload_b64.as_bytes())?;
        let header = hex::decode(&header_hex).unwrap_or_default();

        // Header scan: brute-force every overlapping 8-byte window over the
        // unparsed regions (8-15 and 24-end). Print the offset of any hit so
        // we can confirm a fixed slot.
        let mut hdr_tals = Vec::new();
        let mut hdr_abls = Vec::new();
        let mut hdr_offsets: Vec<(usize, String)> = Vec::new();
        if header.len() >= 8 {
            for start in 0..=header.len() - 8 {
                // Skip the known content-GUID slot (0..8) and template-GUID slot (16..24).
                if start == 0 {
                    continue;
                }
                if (16..24).contains(&start) {
                    continue;
                }
                let bytes: [u8; 8] = header[start..start + 8].try_into().unwrap();
                let guid_le = format!("{:016X}", u64::from_le_bytes(bytes));
                let guid_be = format!("{:016X}", u64::from_be_bytes(bytes));
                for guid in [&guid_le, &guid_be] {
                    if let Some((kind, target_fqn)) = crew.get(guid) {
                        let entry = format!("{} -> {} (offset {})", guid, target_fqn, start);
                        if *kind == "tal" {
                            hdr_tals.push(entry.clone());
                        } else {
                            hdr_abls.push(entry.clone());
                        }
                        hdr_offsets.push((start, target_fqn.clone()));
                    }
                }
            }
        }
        summary_header_hits.insert(fqn.clone(), (hdr_tals, hdr_abls, hdr_offsets));

        // a. cf-marker GUIDs.
        let mut cf_guids: BTreeSet<String> = BTreeSet::new();
        let mut i = 0;
        while i + 9 <= payload.len() {
            if payload[i] == 0xCF {
                let bytes: [u8; 8] = payload[i + 1..i + 9].try_into().unwrap();
                let guid = format!("{:016X}", u64::from_le_bytes(bytes));
                cf_guids.insert(guid);
                i += 9;
            } else {
                i += 1;
            }
        }

        // b. Brute-force every 8-byte window (overlapping).
        let mut window_guids: BTreeSet<String> = BTreeSet::new();
        if payload.len() >= 8 {
            for start in 0..=payload.len() - 8 {
                let bytes: [u8; 8] = payload[start..start + 8].try_into().unwrap();
                let guid = format!("{:016X}", u64::from_le_bytes(bytes));
                if crew.contains_key(&guid) {
                    window_guids.insert(guid);
                }
            }
        }

        let mut cf_tals = Vec::new();
        let mut cf_abls = Vec::new();
        for g in &cf_guids {
            if let Some((kind, target_fqn)) = crew.get(g) {
                let entry = format!("{} -> {}", g, target_fqn);
                if *kind == "tal" {
                    cf_tals.push(entry);
                } else {
                    cf_abls.push(entry);
                }
                *crew_object_hit_count.entry(target_fqn.clone()).or_default() += 1;
            }
        }

        let mut win_tals = Vec::new();
        let mut win_abls = Vec::new();
        for g in &window_guids {
            if let Some((kind, target_fqn)) = crew.get(g) {
                let entry = format!("{} -> {}", g, target_fqn);
                if *kind == "tal" {
                    win_tals.push(entry);
                } else {
                    win_abls.push(entry);
                }
            }
        }

        summary_cf_hits.insert(fqn.clone(), (cf_tals, cf_abls));
        summary_window_hits.insert(fqn, (win_tals, win_abls));
    }

    // 3. Report.
    println!("\n=== per-companion: cf-marker matches ===");
    for (fqn, (tals, abls)) in &summary_cf_hits {
        println!("\n{}", fqn);
        println!("  tal.spvp.crew matches ({})", tals.len());
        for t in tals {
            println!("    {}", t);
        }
        println!("  abl.spvp.crew matches ({})", abls.len());
        for a in abls {
            println!("    {}", a);
        }
    }

    println!("\n=== per-companion: brute-force 8-byte window matches ===");
    println!("(use this if cf-marker count looks wrong)");
    for (fqn, (tals, abls)) in &summary_window_hits {
        let total = tals.len() + abls.len();
        if total == 0 {
            println!("\n{}  -- NO WINDOW HITS", fqn);
        } else {
            println!("\n{}  ({} tal + {} abl)", fqn, tals.len(), abls.len());
            for t in tals {
                println!("    {}", t);
            }
            for a in abls {
                println!("    {}", a);
            }
        }
    }

    println!("\n=== per-companion: HEADER scan (bytes 8-15, 24-41) ===");
    for (fqn, (tals, abls, offsets)) in &summary_header_hits {
        let total = tals.len() + abls.len();
        if total == 0 {
            println!("\n{}  -- NO HEADER HITS", fqn);
        } else {
            println!("\n{}  ({} tal + {} abl)", fqn, tals.len(), abls.len());
            for t in tals {
                println!("    {}", t);
            }
            for a in abls {
                println!("    {}", a);
            }
            let _ = offsets;
        }
    }

    println!("\n=== raw header dumps (so we can eyeball unparsed regions) ===");
    let mut stmt = conn.prepare(
        "SELECT fqn, json_extract(json, '$.header_hex') FROM objects \
         WHERE fqn LIKE 'npc.companion.spvp.%' ORDER BY fqn",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows.filter_map(|r| r.ok()) {
        let (fqn, header_hex) = row;
        println!("\n  {}", fqn);
        println!("    full   : {}", header_hex);
        let bytes = hex::decode(&header_hex).unwrap_or_default();
        if bytes.len() >= 8 {
            println!(
                "    [00-07]: {} (content GUID, LE u64)",
                hex::encode_upper(&bytes[0..8])
            );
        }
        if bytes.len() >= 16 {
            println!(
                "    [08-15]: {} (UNPARSED)",
                hex::encode_upper(&bytes[8..16])
            );
        }
        if bytes.len() >= 24 {
            println!(
                "    [16-23]: {} (template GUID)",
                hex::encode_upper(&bytes[16..24])
            );
        }
        if bytes.len() > 24 {
            println!(
                "    [24-{:02}]: {} (UNPARSED)",
                bytes.len() - 1,
                hex::encode_upper(&bytes[24..])
            );
        }
    }

    println!("\n=== summary ===");
    let total = summary_cf_hits.len();
    let with_2tal_1abl_cf = summary_cf_hits
        .values()
        .filter(|(t, a)| t.len() == 2 && a.len() == 1)
        .count();
    let with_2tal_1abl_win = summary_window_hits
        .values()
        .filter(|(t, a)| t.len() == 2 && a.len() == 1)
        .count();
    println!("companions probed: {}", total);
    println!(
        "companions with exactly 2 tal + 1 abl crew refs (cf marker): {}/{}",
        with_2tal_1abl_cf, total
    );
    println!(
        "companions with exactly 2 tal + 1 abl crew refs (window):    {}/{}",
        with_2tal_1abl_win, total
    );

    println!("\ntop crew objects referenced (cf marker, by hit count):");
    let mut hit_counts: Vec<_> = crew_object_hit_count.iter().collect();
    hit_counts.sort_by(|a, b| b.1.cmp(a.1));
    for (fqn, count) in hit_counts.iter().take(20) {
        println!("  {:3}x  {}", count, fqn);
    }

    Ok(())
}
