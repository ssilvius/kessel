//! One-shot: dump the full payload of dis.* objects as hex + decode CF E0 list.
use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut input_dir = PathBuf::from(".");
    let mut hash_path = PathBuf::from("/tmp/hashes_filename.txt");
    let mut target_fqn = String::new();
    let args: Vec<String> = std::env::args().collect();
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
            "-f" => {
                target_fqn = args[i + 1].clone();
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
            let path = hashes.get(entry.filename_hash);
            if path.map(|p| p.contains("/buckets/")).unwrap_or(false) {
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
                    if !target_fqn.is_empty() && obj.fqn != target_fqn {
                        continue;
                    }
                    if target_fqn.is_empty() && !obj.fqn.starts_with("dis.") {
                        continue;
                    }
                    println!("\n=== {} ({} bytes) ===", obj.fqn, obj.payload.len());
                    for row in 0..obj.payload.len().div_ceil(16) {
                        let s = row * 16;
                        let e = (s + 16).min(obj.payload.len());
                        let hex: String = obj.payload[s..e]
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let ascii: String = obj.payload[s..e]
                            .iter()
                            .map(|&b| {
                                if (32..127).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!("  {s:04X}: {hex:<48} {ascii}");
                    }
                }
            }
        }
    }
    Ok(())
}
