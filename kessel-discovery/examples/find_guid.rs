//! Find which PBUK object(s) contain a given content/template GUID as raw
//! bytes. Useful when a CF E0 reference does not resolve to an extracted
//! object -- locates the actual prototype/record that owns it.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.trim().trim_start_matches("0x");
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
    let mut needles: Vec<Vec<u8>> = Vec::new();

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
            "-g" | "--guid" => {
                needles.push(parse_hex(&args[i + 1]));
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
        "Searching {} .tor files for {} GUID needle(s)",
        tor_files.len(),
        needles.len()
    );
    for n in &needles {
        eprintln!(
            "  needle: {}",
            n.iter().map(|b| format!("{b:02X}")).collect::<String>()
        );
    }

    let mut total_objects = 0;
    let mut hits = 0;

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
                total_objects += 1;
                let payload = obj.payload.as_slice();
                for needle in &needles {
                    let mut j = 0;
                    while j + needle.len() <= payload.len() {
                        if &payload[j..j + needle.len()] == needle.as_slice() {
                            println!(
                                "HIT  fqn={}  archive={}  offset={}  needle={}",
                                obj.fqn,
                                tor_path.file_name().unwrap().to_string_lossy(),
                                j,
                                needle
                                    .iter()
                                    .map(|b| format!("{b:02X}"))
                                    .collect::<String>()
                            );
                            hits += 1;
                            break;
                        }
                        j += 1;
                    }
                }
            }
        }
    }

    eprintln!("Scanned {total_objects} objects, {hits} hits");
    Ok(())
}
