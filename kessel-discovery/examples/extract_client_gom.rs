//! Extract /resources/systemgenerated/client.gom to a flat file for analysis.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut out_path = PathBuf::from("/tmp/client.gom.bin");

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
            "-o" => {
                out_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let target = "/resources/systemgenerated/client.gom";
    let target_hash = hashes
        .paths_matching(target)
        .into_iter()
        .find(|(_, p)| p == &&target.to_string())
        .map(|(h, _)| h)
        .ok_or_else(|| anyhow::anyhow!("client.gom not in dict"))?;

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
            if entry.filename_hash == target_hash {
                let data = archive.read_entry(entry)?;
                std::fs::write(&out_path, &data)?;
                eprintln!("wrote {} bytes to {}", data.len(), out_path.display());
                return Ok(());
            }
        }
    }
    anyhow::bail!("client.gom not found");
}
