//! Audit every NODE file in the archive: count present-vs-stale, and for
//! each present NODE classify by header layout. The "6,779 unaccounted"
//! NODEs from the cnv-only scan need to be characterized -- different
//! header (not offset 0x14), different magic, or genuinely different format.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut dump_samples = 8usize;
    let mut dump_kind: Option<String> = None;

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
            "-n" => {
                dump_samples = args[i + 1].parse().unwrap_or(8);
                i += 2;
            }
            "-k" => {
                dump_kind = Some(args[i + 1].clone());
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let prototype_hashes: HashSet<u64> = hashes
        .paths_matching("/resources/systemgenerated/prototypes/")
        .into_iter()
        .map(|(h, _)| h)
        .collect();

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    eprintln!(
        "Auditing {} prototype hashes across {} .tor files",
        prototype_hashes.len(),
        tor_files.len()
    );

    let mut found_in_archive: HashSet<u64> = HashSet::new();
    let mut layout_counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut layout_samples: BTreeMap<&'static str, Vec<(String, String, Vec<u8>)>> =
        BTreeMap::new();

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
            if !prototype_hashes.contains(&entry.filename_hash) {
                continue;
            }
            found_in_archive.insert(entry.filename_hash);
            if entry.compressed_size == 0 {
                let key = "zero_compressed_size";
                *layout_counts.entry(key).or_default() += 1;
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => {
                    let key = "decompress_failed";
                    *layout_counts.entry(key).or_default() += 1;
                    continue;
                }
            };

            // Classify by first 4 bytes magic + FQN-at-0x14 presence.
            let magic = if data.len() >= 4 {
                format!(
                    "{:02X}{:02X}{:02X}{:02X}",
                    data[0], data[1], data[2], data[3]
                )
            } else {
                "TOO_SMALL".to_string()
            };

            let layout: &'static str = if data.len() < 0x14 + 8 {
                "tiny_no_header"
            } else {
                let mut e = 0x14;
                while e < data.len() && e < 0x14 + 256 && data[e] != 0 {
                    e += 1;
                }
                let fqn_str = std::str::from_utf8(&data[0x14..e]).ok();
                match fqn_str {
                    Some(s) if s.starts_with("cnv.") => "cnv_at_0x14",
                    Some(s) if s.contains('.') && s.is_ascii() => "other_fqn_at_0x14",
                    Some(s) if !s.is_empty() && s.is_ascii() => "ascii_no_dot_at_0x14",
                    _ => "non_ascii_at_0x14",
                }
            };
            *layout_counts.entry(layout).or_default() += 1;

            let bucket = layout_samples.entry(layout).or_default();
            if bucket.len() < dump_samples {
                let preview_n = data.len().min(96);
                bucket.push((
                    tor_path.file_name().unwrap().to_string_lossy().to_string(),
                    magic.clone(),
                    data[..preview_n].to_vec(),
                ));
            }
        }
    }

    println!("=== NODE accounting ===");
    println!(
        "  hash dict prototype paths     : {}",
        prototype_hashes.len()
    );
    println!(
        "  found in scanned archives     : {}",
        found_in_archive.len()
    );
    println!(
        "  stale hashes (not in archive) : {}",
        prototype_hashes.len() - found_in_archive.len()
    );
    println!();
    println!("=== layout distribution ===");
    for (layout, count) in &layout_counts {
        println!("  {layout:<24} {count:>10}");
    }
    println!();
    if let Some(kind) = &dump_kind {
        println!("=== samples for layout '{kind}' ===");
        if let Some(samples) = layout_samples.get(kind.as_str()) {
            for (archive, magic, preview) in samples {
                let hex_preview: String = preview
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let ascii_preview: String = preview
                    .iter()
                    .map(|&b| {
                        if (32..127).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("--- {archive}  magic={magic}  size={}", preview.len());
                println!("  hex   = {hex_preview}");
                println!("  ascii = {ascii_preview}");
            }
        } else {
            println!("(no samples for that layout)");
        }
    } else {
        println!("=== samples per layout (first {dump_samples}) ===");
        for (layout, samples) in &layout_samples {
            println!("--- layout: {layout} ({} samples) ---", samples.len());
            for (archive, magic, preview) in samples.iter().take(2) {
                let hex_preview: String = preview
                    .iter()
                    .take(48)
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let ascii_preview: String = preview
                    .iter()
                    .take(48)
                    .map(|&b| {
                        if (32..127).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("  {archive} magic={magic}");
                println!("    hex   {hex_preview}");
                println!("    ascii {ascii_preview}");
            }
        }
    }

    Ok(())
}
