//! Extract prototypes.info and buckets.info to /tmp for analysis.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn main() -> Result<()> {
    let input_dir = PathBuf::from("/Users/seansilvius/swtor/Assets");
    let hash_path = PathBuf::from("/Users/seansilvius/swtor/data/hashes_filename.txt");
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let targets: Vec<(&str, &str)> = vec![
        (
            "/resources/systemgenerated/prototypes.info",
            "/tmp/prototypes.info.bin",
        ),
        (
            "/resources/systemgenerated/buckets.info",
            "/tmp/buckets.info.bin",
        ),
        (
            "/resources/systemgenerated/scriptdef.list",
            "/tmp/scriptdef.list.bin",
        ),
    ];

    for (target, out_path) in &targets {
        let Some(target_hash) = hashes
            .paths_matching(target)
            .into_iter()
            .find(|(_, p)| p == &&target.to_string())
            .map(|(h, _)| h)
        else {
            eprintln!("{}: not in hash dict", target);
            continue;
        };

        let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
            .collect();

        let mut found = false;
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
                if entry.filename_hash == target_hash {
                    let data = archive.read_entry(entry)?;
                    std::fs::write(out_path, &data)?;
                    eprintln!("{}: wrote {} bytes to {}", target, data.len(), out_path);
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            eprintln!("{}: not found", target);
        }
    }
    Ok(())
}
