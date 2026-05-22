//! Extract field-name to hash mappings from /resources/systemgenerated/client.gom.
//! This is the master GOM schema definition file. It contains all type IDs,
//! field hashes, and enum names referenced throughout the corpus.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut needle = String::new();

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
            "-n" => {
                needle = args[i + 1].clone();
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

    let mut data: Vec<u8> = Vec::new();
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
                data = archive.read_entry(entry)?;
                break;
            }
        }
        if !data.is_empty() {
            break;
        }
    }
    if data.is_empty() {
        anyhow::bail!("client.gom not found in archive");
    }

    eprintln!("client.gom: {} bytes", data.len());

    // The file starts with DBLB magic + records.
    // Each record seems to follow pattern:
    //   [4 bytes record-size?]
    //   [4 bytes record-id-hash?]
    //   [4 bytes record-type-hash u64?]
    //   [body: type-dependent]
    //
    // For the GSF damage field at offset ~0x12C00, look at bytes BEFORE the
    // string to find the structural marker.

    // Approach: scan for the needle, print 64 bytes before and 64 bytes after.
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() {
        // Default: scan for several known field strings and print their context
        for name in &[
            "effAction_SPVPWeaponDamage",
            "effAction_SPVPDamage",
            "effAction_WeaponDamage",
            "staWeaponState",
            "scffWeapon",
            "spvpDamage",
        ] {
            scan_print(&data, name.as_bytes(), name);
        }
        return Ok(());
    }

    scan_print(&data, needle_bytes, &needle);
    Ok(())
}

fn scan_print(data: &[u8], needle: &[u8], name: &str) {
    let mut idx = 0;
    let mut found = 0;
    while let Some(pos) = data[idx..].windows(needle.len()).position(|w| w == needle) {
        let abs = idx + pos;
        // Need to check this is a complete NULL-terminated string (not a substring)
        if abs > 0 && data[abs - 1] != 0 {
            idx = abs + 1;
            continue;
        }
        let end = abs + needle.len();
        if end < data.len() && data[end] != 0 {
            idx = abs + 1;
            continue;
        }
        let before_start = abs.saturating_sub(48);
        let after_end = (end + 48).min(data.len());
        let before: String = data[before_start..abs]
            .iter()
            .map(|b| format!("{:02X} ", b))
            .collect();
        let after: String = data[end..after_end]
            .iter()
            .map(|b| format!("{:02X} ", b))
            .collect();
        println!("\n  {} @ offset 0x{:X}:", name, abs);
        println!("    before: ...{}", before);
        println!("    str:    {} (len={})", name, needle.len());
        println!("    after:  {}...", after);
        idx = end;
        found += 1;
        if found >= 3 {
            break;
        }
    }
}
