//! Read prototypes.info (PINF) and produce a histogram of flag values.
//!
//! Each flag byte routes a .node prototype to its schema. Knowing the flag
//! distribution tells us how many prototypes of each KIND exist without
//! decoding any .node file. Output: flag -> count.
//!
//! Usage: ./target/release/pinf_flag_histogram -i ~/swtor/Assets -H /tmp/hashes_filename.txt

use anyhow::{anyhow, Result};
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::prototypes_info;
use std::collections::BTreeMap;
use std::path::PathBuf;

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

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let pinf_path = "/resources/systemgenerated/prototypes.info";
    let pinf_hash = kessel::hash::swtor_filename_hash(pinf_path);

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut pinf_data: Option<Vec<u8>> = None;
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
            if entry.filename_hash == pinf_hash {
                if let Ok(d) = archive.read_entry(entry) {
                    pinf_data = Some(d);
                    break;
                }
            }
        }
        if pinf_data.is_some() {
            break;
        }
    }

    let pinf_bytes = pinf_data.ok_or_else(|| anyhow!("prototypes.info not found in any .tor"))?;
    eprintln!("PINF bytes: {}", pinf_bytes.len());
    let records = prototypes_info::parse(&pinf_bytes)?;
    eprintln!("PINF records: {}", records.len());

    let mut histogram: BTreeMap<u8, u64> = BTreeMap::new();
    for r in &records {
        *histogram.entry(r.flag).or_insert(0) += 1;
    }

    println!("flag_hex\tflag_dec\tcount");
    for (flag, count) in &histogram {
        println!("{:02X}\t{}\t{}", flag, flag, count);
    }
    println!();
    println!("total records: {}", records.len());
    println!("distinct flags: {}", histogram.len());

    Ok(())
}
