//! Dump raw STB entries (pre-grammar) for given id2 values. Used to inspect
//! template tokens `<<N[...]>>` that get stripped at db-insert time.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::stb;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut target_path = String::from("/resources/en-us/str/abl.stb");
    let mut id2s: Vec<u32> = Vec::new();

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
            "-p" | "--path" => {
                target_path = args[i + 1].clone();
                i += 2;
            }
            "--id2" => {
                id2s.push(args[i + 1].parse()?);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;
    let target_hash = hashes
        .paths_matching(&target_path)
        .into_iter()
        .find(|(_, p)| p == &&target_path)
        .map(|(h, _)| h)
        .ok_or_else(|| anyhow::anyhow!("path not in hash dict"))?;

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
            if entry.filename_hash != target_hash {
                continue;
            }
            let data = archive.read_entry(entry)?;
            let stb_file = stb::parse(&data, &target_path)?;
            eprintln!(
                "loaded {} ({} entries) from {}",
                target_path,
                stb_file.entries.len(),
                tor_path.file_name().unwrap().to_string_lossy(),
            );
            for id2 in &id2s {
                let matches: Vec<_> = stb_file.entries.iter().filter(|e| e.id2 == *id2).collect();
                println!("=== id2={id2} ({} entries) ===", matches.len());
                for (i, e) in matches.iter().enumerate() {
                    println!(
                        "  [{i}] id1={} version={} text={:?}",
                        e.id1, e.version, e.text
                    );
                }
            }
            return Ok(());
        }
    }
    Ok(())
}
