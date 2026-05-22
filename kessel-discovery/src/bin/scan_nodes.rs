//! Scan every .node file for raw ASCII needles. Used to detect whether GSF
//! ship hull data lives in NODE files (the ~7K files that aren't `cnv.*`).
//!
//! Usage:
//!   scan_nodes -i ~/swtor/Assets -H ~/swtor/data/hashes_filename.txt -n rycer -n blackbolt -n Hull -n Shield

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::HashSet;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut needles: Vec<String> = Vec::new();
    let mut dump_first_non_cnv = 0usize;

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
            "-n" | "--needle" => {
                needles.push(args[i + 1].clone());
                i += 2;
            }
            "-d" | "--dump" => {
                dump_first_non_cnv = args[i + 1].parse().unwrap_or(5);
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
        "Scanning {} .tor files, {} prototype hashes, {} needles",
        tor_files.len(),
        prototype_hashes.len(),
        needles.len()
    );

    let mut total_nodes = 0u64;
    let mut cnv_nodes = 0u64;
    let mut non_cnv_dumped = 0;

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
            total_nodes += 1;

            let fqn_start = 0x14;
            let is_cnv = if data.len() >= fqn_start + 4 {
                let mut e = fqn_start;
                while e < data.len() && e < fqn_start + 256 && data[e] != 0 {
                    e += 1;
                }
                let fqn = String::from_utf8_lossy(&data[fqn_start..e]);
                fqn.starts_with("cnv.") && fqn.is_ascii()
            } else {
                false
            };
            if is_cnv {
                cnv_nodes += 1;
            }

            // For non-cnv nodes, optionally dump first 96 bytes + any printable strings near start
            if !is_cnv && non_cnv_dumped < dump_first_non_cnv {
                non_cnv_dumped += 1;
                let preview_len = data.len().min(96);
                let hex_preview: String = data[..preview_len]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let ascii_preview: String = data[..preview_len]
                    .iter()
                    .map(|&b| {
                        if (32..127).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!(
                    "NON-CNV NODE: archive={} hash={:016X} size={} hex={}\n   ascii={}",
                    tor_path.file_name().unwrap().to_string_lossy(),
                    entry.filename_hash,
                    data.len(),
                    hex_preview,
                    ascii_preview
                );

                // also extract any ASCII strings (>=4 chars) in first 4KB
                let scan_len = data.len().min(4096);
                let mut strings = Vec::new();
                let mut k = 0;
                while k < scan_len {
                    let start = k;
                    while k < scan_len && data[k].is_ascii_graphic() && data[k] != 0 {
                        k += 1;
                    }
                    if k - start >= 4 {
                        if let Ok(s) = std::str::from_utf8(&data[start..k]) {
                            strings.push(s.to_string());
                        }
                    }
                    k += 1;
                }
                println!(
                    "   strings({}): {}",
                    strings.len(),
                    strings
                        .iter()
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
            }

            // Needle search across whole payload
            for needle in &needles {
                let needle_bytes = needle.as_bytes();
                if needle_bytes.is_empty() {
                    continue;
                }
                let mut k = 0;
                while k + needle_bytes.len() <= data.len() {
                    if &data[k..k + needle_bytes.len()] == needle_bytes {
                        println!(
                            "HIT  needle={} archive={} node_hash={:016X} offset={} size={}",
                            needle,
                            tor_path.file_name().unwrap().to_string_lossy(),
                            entry.filename_hash,
                            k,
                            data.len()
                        );
                        break;
                    }
                    k += 1;
                }
            }
        }
    }

    eprintln!(
        "\nTotal nodes scanned: {total_nodes}, cnv-shaped: {cnv_nodes}, non-cnv: {}",
        total_nodes - cnv_nodes
    );
    Ok(())
}
