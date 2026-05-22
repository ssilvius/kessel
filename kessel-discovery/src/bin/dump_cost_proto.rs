//! Dump the raw bytes of a singleton prototype (e.g. scFFComponentsCostPrototype)
//! for hand inspection. Reads .bkt PBUK buckets, finds the named object, dumps
//! the payload as hex+ascii.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

fn hexdump(bytes: &[u8], limit: usize) {
    let n = bytes.len().min(limit);
    for i in (0..n).step_by(16) {
        let chunk = &bytes[i..(i + 16).min(n)];
        let h: String = chunk.iter().map(|b| format!("{b:02X} ")).collect();
        let a: String = chunk
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  {i:04x}  {h:<48}  {a}");
    }
    if bytes.len() > n {
        println!("  ... ({} more bytes)", bytes.len() - n);
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut target_fqn = String::new();
    let mut limit = 400usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" => {
                hash_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "-f" => {
                target_fqn = args[i + 1].clone();
                i += 2;
            }
            "-n" => {
                limit = args[i + 1].parse()?;
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
            if !path.map(|p| p.contains("/buckets/")).unwrap_or(false) {
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
                if obj.fqn == target_fqn {
                    // Read guid + template from header bytes 0-7 and 16-23
                    let mut guid_str = String::new();
                    let mut tmpl_str = String::new();
                    if obj.header.len() >= 24 {
                        let g: [u8; 8] = obj.header[0..8].try_into().unwrap();
                        let t: [u8; 8] = obj.header[16..24].try_into().unwrap();
                        guid_str = format!("{:016X}", u64::from_le_bytes(g));
                        tmpl_str = format!("{:016X}", u64::from_le_bytes(t));
                    }
                    println!(
                        "=== {} ({} bytes)  guid={}  template={} ===",
                        obj.fqn,
                        obj.payload.len(),
                        guid_str,
                        tmpl_str
                    );
                    hexdump(&obj.payload, limit);
                    return Ok(());
                }
            }
        }
    }
    eprintln!("not found: {target_fqn}");
    Ok(())
}
