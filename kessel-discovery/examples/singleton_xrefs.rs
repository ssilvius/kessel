//! Walk every zero-dot singleton prototype and extract FQN-shaped strings
//! from its payload. Group by prefix and report which prefixes appear in
//! singleton xrefs but are NOT in kessel's extraction whitelist.
//!
//! Output: TSV (prefix, total_refs, distinct_singletons_referencing,
//! sample_fqn) plus a summary of "excluded sources" -- prefixes referenced
//! by at least one singleton but filtered by should_extract_object.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const EXTRACTED_PREFIXES: &[&str] = &[
    "abl", "tal", "itm", "npc", "schem", "qst", "cdx", "ach", "mpn", "pkg", "loot", "rew", "cnv",
    "apc", "class", "enc", "spn", "plc", "epp",
];

fn extract_fqn_strings(payload: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < payload.len() {
        let start = i;
        while i < payload.len()
            && (payload[i].is_ascii_alphanumeric() || payload[i] == b'.' || payload[i] == b'_')
        {
            i += 1;
        }
        let len = i - start;
        if (4..=200).contains(&len) {
            if let Ok(s) = std::str::from_utf8(&payload[start..i]) {
                if s.contains('.')
                    && s.split('.')
                        .next()
                        .map(|p| {
                            p.chars()
                                .all(|c| c.is_ascii_alphabetic() && c.is_ascii_lowercase())
                        })
                        .unwrap_or(false)
                    && !s.starts_with('.')
                    && !s.ends_with('.')
                {
                    out.push(s.to_string());
                }
            }
        }
        i += 1;
    }
    out
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" | "--input" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" | "--hashes" => {
                hash_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    eprintln!("Scanning {} .tor files", tor_files.len());

    // prefix -> (total_ref_count, set_of_singletons_referencing, sample_fqns)
    let mut prefix_stats: BTreeMap<String, (u64, BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    let mut seen_singletons: BTreeSet<String> = BTreeSet::new();

    for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if entry.compressed_size == 0 {
                continue;
            }
            let path = hashes.get(entry.filename_hash);
            let is_bucket = path.map(|p| p.contains("/buckets/")).unwrap_or(false);
            if !is_bucket {
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
                if obj.fqn.contains('.') || obj.fqn.is_empty() {
                    continue;
                }
                if !seen_singletons.insert(obj.fqn.clone()) {
                    continue;
                }
                let strings = extract_fqn_strings(&obj.payload);
                for s in strings {
                    let prefix = s.split('.').next().unwrap_or("").to_string();
                    if prefix.is_empty() {
                        continue;
                    }
                    let entry = prefix_stats.entry(prefix).or_default();
                    entry.0 += 1;
                    entry.1.insert(obj.fqn.clone());
                    if entry.2.len() < 5 {
                        entry.2.insert(s);
                    }
                }
            }
        }
    }

    let extracted: BTreeSet<&str> = EXTRACTED_PREFIXES.iter().copied().collect();

    println!("=== EXCLUDED PREFIXES referenced by singleton payloads ===");
    println!("PREFIX           REF_COUNT  SINGLETONS  SAMPLE");
    println!("---------------- ---------- ----------  ----------------------------------");
    let mut sorted: Vec<_> = prefix_stats.iter().collect();
    sorted.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    for (prefix, (count, singletons, samples)) in &sorted {
        if extracted.contains(prefix.as_str()) {
            continue;
        }
        let sample = samples.iter().next().cloned().unwrap_or_default();
        println!(
            "{:<16} {:>10} {:>10}  {}",
            prefix,
            count,
            singletons.len(),
            sample
        );
    }

    println!();
    println!("=== EXTRACTED PREFIXES (for comparison) ===");
    for (prefix, (count, singletons, samples)) in &sorted {
        if !extracted.contains(prefix.as_str()) {
            continue;
        }
        let sample = samples.iter().next().cloned().unwrap_or_default();
        println!(
            "{:<16} {:>10} {:>10}  {}",
            prefix,
            count,
            singletons.len(),
            sample
        );
    }

    eprintln!("\nScanned {} singletons", seen_singletons.len());
    Ok(())
}
