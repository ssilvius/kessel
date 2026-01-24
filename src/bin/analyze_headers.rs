//! Analyze per-file headers in MYP archives for DDS files
//!
//! Run with: cargo run --bin analyze-headers -- -i ~/swtor/assets -H ~/swtor/data/hashes_filename.txt
//!
//! This script reads the per-file headers (the bytes BEFORE the actual file data)
//! for DDS texture files to understand what metadata they contain.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

// Import from parent crate
use kessel::hash::HashDictionary;
use kessel::myp::Archive;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;

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
            _ => i += 1,
        }
    }

    // Load hash dictionary
    let mut hash_dict = HashDictionary::new();
    if let Some(hp) = &hash_path {
        let count = hash_dict.load(hp)?;
        println!("Loaded {} hash entries", count);
    }

    // Find DDS file hashes
    let dds_hashes: HashMap<u64, String> = hash_dict
        .paths_matching("/resources/gfx/icons/abl_")
        .into_iter()
        .filter(|(_, path)| path.ends_with(".dds"))
        .map(|(hash, path)| (hash, path.clone()))
        .collect();

    println!("Found {} ability icon hashes", dds_hashes.len());

    // Find all .tor files
    let mut tor_files: Vec<PathBuf> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "tor"))
        .collect();
    tor_files.sort();

    println!("Found {} archive files", tor_files.len());

    // Collect header samples
    let mut header_samples: Vec<(String, Vec<u8>, u32)> = Vec::new();
    let mut header_sizes: HashMap<u32, usize> = HashMap::new();

    for tor_path in &tor_files {
        match Archive::open(tor_path) {
            Ok(mut archive) => {
                let entries: Vec<_> = match archive.entries() {
                    Ok(iter) => iter.cloned().collect(),
                    Err(_) => continue,
                };

                for entry in &entries {
                    // Check if this is a DDS file we care about
                    if let Some(path) = dds_hashes.get(&entry.filename_hash) {
                        // Read the per-file header
                        if let Ok(header) = archive.read_entry_header(entry) {
                            *header_sizes.entry(entry.header_size).or_insert(0) += 1;

                            if header_samples.len() < 50 {
                                header_samples.push((path.clone(), header, entry.header_size));
                            }
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }

    println!("\n=== Header Size Distribution ===");
    let mut sizes: Vec<_> = header_sizes.iter().collect();
    sizes.sort_by_key(|(size, _)| *size);
    for (size, count) in sizes {
        println!("  {} bytes: {} files", size, count);
    }

    println!("\n=== Sample Headers ===");
    for (path, header, size) in &header_samples {
        println!("\nFile: {}", path);
        println!("Header size: {} bytes", size);

        if header.is_empty() {
            println!("  (no header)");
            continue;
        }

        // Print raw hex
        println!("Raw hex:");
        for (i, chunk) in header.chunks(16).enumerate() {
            print!("  {:04x}: ", i * 16);
            for byte in chunk {
                print!("{:02x} ", byte);
            }
            // Print ASCII representation
            print!(" | ");
            for byte in chunk {
                if *byte >= 0x20 && *byte < 0x7f {
                    print!("{}", *byte as char);
                } else {
                    print!(".");
                }
            }
            println!();
        }

        // Try to parse common header fields
        if header.len() >= 8 {
            println!("\nPossible interpretations:");

            // First 4 bytes as u32
            let first_u32 = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            println!("  Bytes 0-3 as u32 LE: {} (0x{:08x})", first_u32, first_u32);

            // First 6 bytes as potential GUID (like GOM objects)
            if header.len() >= 6 {
                let guid_bytes = &header[0..6];
                println!(
                    "  Bytes 0-5 as GUID: {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    guid_bytes[0],
                    guid_bytes[1],
                    guid_bytes[2],
                    guid_bytes[3],
                    guid_bytes[4],
                    guid_bytes[5]
                );
            }

            // First 8 bytes as u64
            if header.len() >= 8 {
                let first_u64 = u64::from_le_bytes([
                    header[0], header[1], header[2], header[3], header[4], header[5], header[6],
                    header[7],
                ]);
                println!(
                    "  Bytes 0-7 as u64 LE: {} (0x{:016x})",
                    first_u64, first_u64
                );
            }

            // Check for magic numbers
            if &header[0..4] == b"DDS " {
                println!("  STARTS WITH DDS MAGIC - header contains DDS header itself!");
            }

            // Look for embedded strings
            let header_str = String::from_utf8_lossy(header);
            if header_str.contains("abl_") {
                println!("  Contains 'abl_' string!");
            }
            if header_str.contains("gfx") || header_str.contains("icons") {
                println!("  Contains path fragment!");
            }
        }
    }

    Ok(())
}
