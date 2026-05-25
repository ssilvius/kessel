//! Probe tagTablePrototype singleton: decode each CE-prefixed record
//! into (7-byte tag hash, tag FQN) pairs and verify the count.
//!
//! Format: <CE> <7-byte hash> <1-byte length> <length bytes of tag FQN>
//!
//! Usage: ./target/release/probe_tag_table -i ~/swtor/Assets -H /tmp/hashes_filename.txt

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path = PathBuf::from("/tmp/hashes_filename.txt");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" => {
                hash_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut payload: Option<Vec<u8>> = None;
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
            let path = hashes.get(entry.filename_hash);
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
            let objects = match pbuk::parse(&data) {
                Ok(o) => o,
                Err(_) => continue,
            };
            for obj in objects {
                if obj.fqn == "tagTablePrototype" {
                    payload = Some(obj.payload);
                    break 'outer;
                }
            }
        }
    }

    let payload = payload.ok_or_else(|| anyhow::anyhow!("tagTablePrototype not found"))?;
    eprintln!("tagTablePrototype payload: {} bytes", payload.len());

    // Walk CE-prefixed records: [CE] [7-byte hash] [1-byte len] [len bytes string]
    let mut tags: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let mut malformed = 0u64;
    while i < payload.len() {
        if payload[i] != 0xCE {
            i += 1;
            continue;
        }
        if i + 9 > payload.len() {
            break;
        }
        let hash_bytes = &payload[i + 1..i + 8];
        let len = payload[i + 8] as usize;
        let str_start = i + 9;
        let str_end = str_start + len;
        if str_end > payload.len() {
            malformed += 1;
            i += 1;
            continue;
        }
        let s = match std::str::from_utf8(&payload[str_start..str_end]) {
            Ok(s) => s,
            Err(_) => {
                malformed += 1;
                i += 1;
                continue;
            }
        };
        if !s.starts_with("tag.") {
            // Not a tag record -- skip
            i += 1;
            continue;
        }
        let hash_hex: String = hash_bytes.iter().map(|b| format!("{b:02X}")).collect();
        tags.push((hash_hex, s.to_string()));
        i = str_end;
    }

    eprintln!(
        "decoded {} tag records ({} malformed CE matches skipped)",
        tags.len(),
        malformed
    );
    eprintln!();
    eprintln!("--- first 8 ---");
    for (h, s) in tags.iter().take(8) {
        eprintln!("  {} | {}", h, s);
    }
    eprintln!();
    eprintln!("--- last 4 ---");
    for (h, s) in tags.iter().rev().take(4) {
        eprintln!("  {} | {}", h, s);
    }
    eprintln!();
    // Bucket by 2nd FQN segment (after "tag.")
    use std::collections::BTreeMap;
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    for (_h, s) in &tags {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() >= 2 {
            *by_kind.entry(parts[1].to_string()).or_insert(0) += 1;
        }
    }
    eprintln!("--- by 2nd segment ---");
    let mut sorted: Vec<_> = by_kind.into_iter().collect();
    sorted.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (k, c) in sorted {
        eprintln!("  {:>6} {}", c, k);
    }

    Ok(())
}
