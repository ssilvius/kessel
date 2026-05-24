//! Sample one PBUK object per FQN prefix.
//!
//! For each distinct FQN prefix found in PBUK buckets, captures the first
//! object: prefix, total count, sample FQN, sample payload size, and the
//! first plaintext ASCII run extracted from the payload as a category hint.
//!
//! Does NOT decode record contents. Output is TSV to stdout.
//!
//! Usage: ./target/release/sample_per_prefix -i ~/swtor/Assets -H /tmp/hashes_filename.txt

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::collections::BTreeMap;
use std::path::PathBuf;

struct Sample {
    count: u64,
    sample_fqn: String,
    payload_size: usize,
    ascii_hint: String,
}

fn extract_ascii_hint(payload: &[u8], max_len: usize) -> String {
    // Find the longest contiguous ASCII printable run (length >= 4).
    let mut best: &[u8] = &[];
    let mut start = 0;
    let mut i = 0;
    while i <= payload.len() {
        let is_end = i == payload.len();
        let printable = !is_end && (payload[i] >= 0x20 && payload[i] < 0x7F);
        if !printable {
            let run = &payload[start..i];
            if run.len() > best.len() && run.len() >= 4 {
                best = run;
            }
            start = i + 1;
        }
        i += 1;
    }
    if best.is_empty() {
        return String::new();
    }
    let s = std::str::from_utf8(best).unwrap_or("").to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
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

    let mut samples: BTreeMap<String, Sample> = BTreeMap::new();

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
                let entry = samples.entry(prefix).or_insert_with(|| Sample {
                    count: 0,
                    sample_fqn: obj.fqn.clone(),
                    payload_size: obj.payload.len(),
                    ascii_hint: extract_ascii_hint(&obj.payload, 80),
                });
                entry.count += 1;
            }
        }
    }

    println!("prefix\tcount\tsample_fqn\tpayload_size\tascii_hint");
    let mut sorted: Vec<_> = samples.into_iter().collect();
    sorted.sort_by_key(|(_, s)| std::cmp::Reverse(s.count));
    for (prefix, s) in &sorted {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            prefix, s.count, s.sample_fqn, s.payload_size, s.ascii_hint
        );
    }

    Ok(())
}
