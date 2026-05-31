//! Search every archive entry (PBUK, NODE, .dat, anything decompressible)
//! for raw byte needles. Used to find a known stat value's location.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        let byte = u8::from_str_radix(&format!("{}{}", chars[i], chars[i + 1]), 16).unwrap();
        out.push(byte);
        i += 2;
    }
    out
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut needles: Vec<(String, Vec<u8>)> = Vec::new();
    let mut max_hits_per_needle = 80usize;

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
                let label = args[i + 1].clone();
                let bytes = parse_hex(&args[i + 2]);
                needles.push((label, bytes));
                i += 3;
            }
            "-m" => {
                max_hits_per_needle = args[i + 1].parse().unwrap_or(80);
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
        "Searching {} .tor files for {} needles",
        tor_files.len(),
        needles.len()
    );
    for (label, bytes) in &needles {
        let hex: String = bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("  {label:<20} {hex}");
    }

    let mut hits_per_needle: Vec<usize> = vec![0; needles.len()];

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
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let path = hashes
                .get(entry.filename_hash)
                .map(|s| s.as_str())
                .unwrap_or("(unknown)");

            for (idx, (label, needle)) in needles.iter().enumerate() {
                if hits_per_needle[idx] >= max_hits_per_needle {
                    continue;
                }
                if needle.is_empty() {
                    continue;
                }
                let mut k = 0;
                while k + needle.len() <= data.len() {
                    if &data[k..k + needle.len()] == needle.as_slice() {
                        println!(
                            "HIT  needle={}  archive={}  path={}  offset={}  size={}",
                            label,
                            tor_path.file_name().unwrap().to_string_lossy(),
                            path,
                            k,
                            data.len()
                        );
                        hits_per_needle[idx] += 1;
                        break;
                    }
                    k += 1;
                }
            }
        }
    }

    eprintln!();
    for (idx, (label, _)) in needles.iter().enumerate() {
        eprintln!(
            "  {label:<20} {} hits (capped at {})",
            hits_per_needle[idx], max_hits_per_needle
        );
    }
    Ok(())
}
