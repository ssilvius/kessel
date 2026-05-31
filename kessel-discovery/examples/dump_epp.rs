//! Dump a single archive entry by filename-hash. Used to inspect .epp
//! files (gamedata/epp/spvp/<ability>/<stage>.epp) for named property
//! schemas. Outputs hex+ascii view + a token-walked decode.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn hexdump(bytes: &[u8]) {
    for i in (0..bytes.len()).step_by(16) {
        let chunk = &bytes[i..(i + 16).min(bytes.len())];
        let hex_part: String = chunk.iter().map(|b| format!("{b:02X} ")).collect();
        let ascii_part: String = chunk
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  {i:04x}  {hex_part:<48}  {ascii_part}");
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut target_path: Option<String> = None;

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
                target_path = Some(args[i + 1].clone());
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let target_path = target_path.expect("--path required");
    let target_hash = hashes
        .paths_matching(&target_path)
        .into_iter()
        .find(|(_, p)| p == &&target_path)
        .map(|(h, _)| h)
        .ok_or_else(|| anyhow::anyhow!("path not in hash dict: {target_path}"))?;

    eprintln!("target_hash = 0x{target_hash:016X}  path = {target_path}");

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
            println!("=== {} ({} bytes) ===", target_path, data.len());
            hexdump(&data);
            return Ok(());
        }
    }

    eprintln!("not found in any .tor");
    Ok(())
}
