//! Bulk dump the first N bytes of every /resources/systemgenerated/compilednative/* entry
//! to /tmp/scpt-headers/<numeric_id>.bin. Walks every .tor archive once.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut out_dir = PathBuf::from("/tmp/scpt-headers");
    let mut head_size: usize = 256;
    let mut full = false;

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
            "-o" | "--out" => {
                out_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-n" | "--bytes" => {
                head_size = args[i + 1].parse().unwrap_or(256);
                i += 2;
            }
            "--full" => {
                full = true;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    // Build map: file_hash -> numeric_id (filename last segment)
    let mut target_hashes: HashMap<u64, String> = HashMap::new();
    for (h, p) in hashes.paths_matching("/compilednative/") {
        if let Some(name) = p.rsplit('/').next() {
            target_hashes.insert(h, name.to_string());
        }
    }
    eprintln!("targets: {} compilednative entries", target_hashes.len());

    fs::create_dir_all(&out_dir)?;

    let tor_files: Vec<_> = fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut written: u64 = 0;
    let mut sizes_seen: HashMap<String, usize> = HashMap::new();

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
            let Some(numeric_id) = target_hashes.get(&entry.filename_hash) else {
                continue;
            };
            if entry.compressed_size == 0 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("failed read {}: {}", numeric_id, e);
                    continue;
                }
            };
            // Skip duplicates - take the first occurrence
            if sizes_seen.contains_key(numeric_id) {
                continue;
            }
            sizes_seen.insert(numeric_id.clone(), data.len());

            let bytes_to_write = if full {
                data.as_slice()
            } else {
                &data[..head_size.min(data.len())]
            };
            let out_path = out_dir.join(format!("{}.bin", numeric_id));
            fs::write(&out_path, bytes_to_write)?;
            written += 1;
            if written % 100 == 0 {
                eprintln!("written {} files...", written);
            }
        }
    }
    eprintln!("done. wrote {} files to {}", written, out_dir.display());

    // Emit a sizes manifest
    let mut manifest = String::new();
    manifest.push_str("numeric_id\tsize_bytes\n");
    let mut keys: Vec<_> = sizes_seen.keys().collect();
    keys.sort();
    for k in keys {
        manifest.push_str(&format!("{}\t{}\n", k, sizes_seen[k]));
    }
    fs::write(out_dir.join("_sizes.tsv"), manifest)?;
    eprintln!("wrote _sizes.tsv");
    Ok(())
}
