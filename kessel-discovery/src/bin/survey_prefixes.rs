//! Survey: enumerate every distinct FQN prefix found in PBUK bucket objects.
//!
//! Usage: ./target/release/survey_prefixes -i ~/swtor/Assets -H ~/swtor/data/hashes_filename.txt
//!
//! Closes part of issue #54. Output: prefix -> count, sorted descending.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::collections::BTreeMap;
use std::path::PathBuf;

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

    eprintln!("Scanning {} .tor files for FQN prefixes", tor_files.len());

    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;

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
                let prefix = obj.fqn.split('.').next().unwrap_or("").to_string();
                if prefix.is_empty() {
                    continue;
                }
                *counts.entry(prefix).or_insert(0) += 1;
                total += 1;
            }
        }
    }

    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    println!("Total objects scanned: {total}");
    println!();
    println!("{:<16} {:>10}", "PREFIX", "COUNT");
    println!("{:-<16} {:->10}", "", "");
    for (prefix, count) in &sorted {
        println!("{prefix:<16} {count:>10}");
    }
    println!();
    println!("Distinct prefixes: {}", sorted.len());

    Ok(())
}
