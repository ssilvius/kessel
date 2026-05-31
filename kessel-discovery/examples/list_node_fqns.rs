//! Walk every .node file in /resources/systemgenerated/prototypes/, extract
//! the FQN at offset 0x14 (PROT header), and print FQN -> count by prefix.
//!
//! Used to discover what NODE prototypes exist beyond `cnv.*` -- specifically
//! looking for ship hull / GSF base-stat data that may live here rather than
//! in PBUK buckets.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut grep: Option<String> = None;

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
            "-g" | "--grep" => {
                grep = Some(args[i + 1].clone());
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let prototype_hashes: HashSet<u64> = hashes
        .paths_matching("/resources/systemgenerated/prototypes/")
        .into_iter()
        .map(|(h, _)| h)
        .collect();

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    eprintln!(
        "Scanning {} .tor files, {} prototype hashes",
        tor_files.len(),
        prototype_hashes.len()
    );

    let mut prefix_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    let mut sample_fqns: BTreeMap<String, Vec<String>> = BTreeMap::new();

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
            if !prototype_hashes.contains(&entry.filename_hash) {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let fqn_start = 0x14;
            if data.len() < fqn_start + 4 {
                continue;
            }
            let mut fqn_end = fqn_start;
            while fqn_end < data.len() && fqn_end < fqn_start + 256 && data[fqn_end] != 0 {
                fqn_end += 1;
            }
            let fqn = String::from_utf8_lossy(&data[fqn_start..fqn_end]).to_string();
            if fqn.is_empty() || !fqn.is_ascii() {
                continue;
            }
            total += 1;
            let prefix = fqn.split('.').next().unwrap_or("").to_string();
            *prefix_counts.entry(prefix.clone()).or_default() += 1;
            let bucket = sample_fqns.entry(prefix).or_default();
            if bucket.len() < 8 {
                bucket.push(fqn.clone());
            }
            if let Some(g) = &grep {
                if fqn.to_lowercase().contains(&g.to_lowercase()) {
                    println!("{}", fqn);
                }
            }
        }
    }

    if grep.is_none() {
        println!("PREFIX                        COUNT");
        println!("------------------------------ ----------");
        let mut sorted: Vec<_> = prefix_counts.iter().collect();
        sorted.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        for (p, c) in sorted {
            println!("{p:<30} {c:>10}");
        }
        eprintln!("\nTotal NODE FQNs: {total}");
        eprintln!("\nSample per prefix (first 8):");
        for (prefix, samples) in &sample_fqns {
            eprintln!("  {prefix}:");
            for s in samples {
                eprintln!("    {s}");
            }
        }
    }
    Ok(())
}
