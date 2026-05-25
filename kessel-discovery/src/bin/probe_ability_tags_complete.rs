//! Final ability<->tag cross-reference: scan ALL objects (canonical + non-canonical),
//! roll up to abl/tal by FQN. Confirm Massacre = 9 per parsely.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut hd = HashDictionary::new();
    hd.load(&PathBuf::from(
        "/Users/seansilvius/.cache/kessel/hashes_filename.txt",
    ))?;
    let tor_files: Vec<_> = std::fs::read_dir(&PathBuf::from("/Users/seansilvius/swtor/Assets"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();
    let mut tp_bytes: Option<Vec<u8>> = None;
    'outer: for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            let path = hd.get(entry.filename_hash);
            if !path.map(|p| p.contains("/buckets/")).unwrap_or(false) {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if !pbuk::is_pbuk(&data) {
                continue;
            }
            let objs = match pbuk::parse(&data) {
                Ok(o) => o,
                Err(_) => continue,
            };
            for obj in objs {
                if obj.fqn == "tagTablePrototype" {
                    tp_bytes = Some(obj.payload);
                    break 'outer;
                }
            }
        }
    }
    let tp = tp_bytes.unwrap();
    let needle = b"tag.";
    let mut h7: HashMap<[u8; 7], String> = HashMap::new();
    let mut h8: HashMap<[u8; 8], String> = HashMap::new();
    let mut idx = 0;
    while idx + 4 <= tp.len() {
        if &tp[idx..idx + 4] == needle {
            let mut end = idx;
            while end < tp.len() && tp[end] >= 0x20 && tp[end] < 0x7F && tp[end] != b' ' {
                end += 1;
            }
            if idx >= 10 && tp[idx - 1] as usize == (end - idx) {
                let lp = idx - 1;
                let name = std::str::from_utf8(&tp[idx..end])
                    .unwrap_or("?")
                    .to_string();
                if tp[lp - 8] == 0xCE {
                    let mut h = [0u8; 7];
                    h.copy_from_slice(&tp[lp - 7..lp]);
                    h7.insert(h, name);
                } else if tp[lp - 9] == 0xCF {
                    let mut h = [0u8; 8];
                    h.copy_from_slice(&tp[lp - 8..lp]);
                    h8.insert(h, name);
                }
            }
            idx = end;
        } else {
            idx += 1;
        }
    }

    let conn = Connection::open("/tmp/spice-173.sqlite")?;
    // Scan ALL rows (canonical + non-canonical), group by FQN.
    let mut stmt = conn.prepare("SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE fqn LIKE 'abl.%' OR fqn LIKE 'tal.%'")?;
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    eprintln!(
        "scanning {} abl/tal rows (canonical + non-canonical)",
        rows.len()
    );

    let mut tags_per_fqn: HashMap<String, HashSet<String>> = HashMap::new();
    for (fqn, b64) in &rows {
        let Some(b64) = b64 else { continue };
        let Ok(p) = B64.decode(b64) else { continue };
        let entry = tags_per_fqn.entry(fqn.clone()).or_default();
        let mut i = 0;
        while i + 7 <= p.len() {
            let mut h = [0u8; 7];
            h.copy_from_slice(&p[i..i + 7]);
            if let Some(n) = h7.get(&h) {
                entry.insert(n.clone());
            }
            i += 1;
        }
        i = 0;
        while i + 8 <= p.len() {
            let mut h = [0u8; 8];
            h.copy_from_slice(&p[i..i + 8]);
            if let Some(n) = h8.get(&h) {
                entry.insert(n.clone());
            }
            i += 1;
        }
    }
    let mut total_edges = 0u64;
    let mut tagged_parents = 0u64;
    for (_f, ts) in &tags_per_fqn {
        if !ts.is_empty() {
            tagged_parents += 1;
            total_edges += ts.len() as u64;
        }
    }
    eprintln!(
        "\ntotal abl/tal FQNs with at least 1 tag: {}",
        tagged_parents
    );
    eprintln!("total tag edges: {}", total_edges);

    // Spot check key abilities
    for target in &[
        "abl.sith_warrior.skill.carnage.massacre",
        "abl.sith_inquisitor.force_surge",
        "abl.bounty_hunter.skill.firebug.flaming_fist",
        "abl.trooper.skill.assault_specialist.mag_bolt",
    ] {
        let ts = tags_per_fqn.get(*target).cloned().unwrap_or_default();
        let mut v: Vec<String> = ts.into_iter().collect();
        v.sort();
        eprintln!("\n{} ({} tags):", target, v.len());
        for t in v {
            eprintln!("  {}", t);
        }
    }
    Ok(())
}
