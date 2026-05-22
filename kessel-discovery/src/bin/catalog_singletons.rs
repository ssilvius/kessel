//! Catalog every zero-dot singleton prototype FQN in PBUK buckets.
//!
//! For each singleton: payload size, ASCII strings (count + samples), repeating
//! marker counts (CF E0 content GUIDs, CF 40 template refs), payload preview.
//!
//! Output: TSV to stdout. One pass over all .tor archives.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::collections::BTreeMap;
use std::path::PathBuf;

struct Catalog {
    payload_size: usize,
    cf_e0_count: usize,
    cf_40_count: usize,
    string_count: usize,
    string_samples: Vec<String>,
    preview: String,
}

fn extract_strings(payload: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < payload.len() {
        let start = i;
        while i < payload.len() && payload[i].is_ascii_graphic() && payload[i] != 0 {
            i += 1;
        }
        if i - start >= 4 {
            if let Ok(s) = std::str::from_utf8(&payload[start..i]) {
                if s.chars().any(|c| c.is_ascii_alphabetic()) {
                    out.push(s.to_string());
                }
            }
        }
        i += 1;
    }
    out
}

fn count_marker(payload: &[u8], marker: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i + marker.len() <= payload.len() {
        if &payload[i..i + marker.len()] == marker {
            count += 1;
            i += marker.len();
        } else {
            i += 1;
        }
    }
    count
}

fn preview_hex(payload: &[u8], n: usize) -> String {
    payload
        .iter()
        .take(n)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
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

    eprintln!(
        "Scanning {} .tor files for zero-dot singleton FQNs",
        tor_files.len()
    );

    let mut catalog: BTreeMap<String, Catalog> = BTreeMap::new();

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
                if catalog.contains_key(&obj.fqn) {
                    continue;
                }
                let payload = obj.payload.as_slice();
                let strings = extract_strings(payload);
                let samples: Vec<String> = strings.iter().take(8).cloned().collect();
                let cat = Catalog {
                    payload_size: payload.len(),
                    cf_e0_count: count_marker(payload, &[0xCF, 0xE0]),
                    cf_40_count: count_marker(payload, &[0xCF, 0x40]),
                    string_count: strings.len(),
                    string_samples: samples,
                    preview: preview_hex(payload, 32),
                };
                catalog.insert(obj.fqn.clone(), cat);
            }
        }
    }

    println!(
        "FQN\tPAYLOAD_SIZE\tCF_E0_COUNT\tCF_40_COUNT\tSTRING_COUNT\tFIRST_32_BYTES\tSTRING_SAMPLES"
    );
    for (fqn, c) in &catalog {
        let samples = c.string_samples.join(" | ");
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            fqn, c.payload_size, c.cf_e0_count, c.cf_40_count, c.string_count, c.preview, samples
        );
    }
    eprintln!("Cataloged {} singletons", catalog.len());
    Ok(())
}
