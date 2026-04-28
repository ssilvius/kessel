//! Dump npp.* GOM objects for format analysis
//!
//! Run with: cargo run --bin dump-npp -- -i ~/swtor/assets -H ~/swtor/data/hashes_filename.txt

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut limit = 20usize;
    let mut prefix_filter = "npp".to_string();
    let mut exact_fqns: Vec<String> = Vec::new();

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
            "-n" | "--limit" => {
                limit = args[i + 1].parse().unwrap_or(20);
                i += 2;
            }
            "-p" | "--prefix" => {
                prefix_filter = args[i + 1].clone();
                i += 2;
            }
            "-f" | "--fqn" => {
                exact_fqns.push(args[i + 1].clone());
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
        .filter(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_str()
                .map(|n| n.starts_with("swtor_main") && !n.contains("anim") && !n.contains("gfx"))
                .unwrap_or(false)
        })
        .collect();

    eprintln!(
        "Scanning {} main .tor files for {}.*",
        tor_files.len(),
        prefix_filter
    );

    let mut found = 0;
    'outer: for tor_path in &tor_files {
        let mut archive = Archive::open(tor_path)?;
        let entries: Vec<_> = archive.entries()?.cloned().collect();

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
                let matches = if !exact_fqns.is_empty() {
                    exact_fqns.iter().any(|f| f == &obj.fqn)
                } else {
                    obj.fqn.split('.').next().unwrap_or("") == prefix_filter
                };
                if !matches {
                    continue;
                }

                println!("=== FQN: {} ===", obj.fqn);
                let (guid_le, guid_be) = {
                    let h = &obj.header;
                    if h.len() >= 8 {
                        let b: [u8; 8] = h[0..8].try_into().unwrap_or([0u8; 8]);
                        (
                            format!("{:016X}", u64::from_le_bytes(b)),
                            format!("{:016X}", u64::from_be_bytes(b)),
                        )
                    } else {
                        ("0000000000000000".into(), "0000000000000000".into())
                    }
                };
                println!("GUID (LE/stored): {guid_le}  (BE/display): {guid_be}");
                println!(
                    "Header ({} bytes): {}",
                    obj.header.len(),
                    obj.header
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );

                // Dump payload hex (first 512 bytes)
                let plen = obj.payload.len().min(512);
                println!(
                    "Payload ({} bytes, showing first {}):",
                    obj.payload.len(),
                    plen
                );
                for row in 0..plen.div_ceil(16) {
                    let start = row * 16;
                    let end = (start + 16).min(plen);
                    let hex: String = obj.payload[start..end]
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let ascii: String = obj.payload[start..end]
                        .iter()
                        .map(|&b| {
                            if (32..127).contains(&b) {
                                b as char
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    println!("  {:04X}: {:<48} {}", start, hex, ascii);
                }

                // Extract printable strings from payload
                let mut strings: Vec<(usize, String)> = Vec::new();
                let mut si = 0;
                while si < obj.payload.len() {
                    if (32..127).contains(&obj.payload[si]) {
                        let mut sj = si;
                        while sj < obj.payload.len() && (32..127).contains(&obj.payload[sj]) {
                            sj += 1;
                        }
                        if sj - si >= 5 {
                            let s = String::from_utf8_lossy(&obj.payload[si..sj]).to_string();
                            strings.push((si, s));
                        }
                        si = sj;
                    } else {
                        si += 1;
                    }
                }
                if !strings.is_empty() {
                    println!("Strings:");
                    for (pos, s) in &strings {
                        println!("  @{pos:04X}: {}", &s[..s.len().min(120)]);
                    }
                }
                println!();

                found += 1;
                if found >= limit {
                    break 'outer;
                }
            }
        }
    }

    eprintln!("Found {found} {prefix_filter}.* objects");
    Ok(())
}
