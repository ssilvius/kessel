//! Audit every dis.* record against the firebug-derived format hypothesis.
//!
//! Per dis.* object:
//!  - codename (length-prefixed string after the CF40 class marker)
//!  - count of CF E0 references
//!  - count of triplet records (signature `02 03 03`)
//!  - signature ability GUID (last CF E0 in payload)
//!  - presence of the expected fixed-bytes transition section
//!
//! Output: TSV, one row per discipline. Surfaces outliers from the 24-mod /
//! 8-tier / 3-choice template.
//!
//! Usage: ./target/release/dis_format_audit -i ~/swtor/Assets -H /tmp/hashes_filename.txt

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

const TRANSITION_MARKER: &[u8] = &[0x03, 0x82, 0xC8, 0x2C, 0x11];

fn count_pattern(payload: &[u8], pattern: &[u8]) -> usize {
    if payload.len() < pattern.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + pattern.len() <= payload.len() {
        if &payload[i..i + pattern.len()] == pattern {
            count += 1;
            i += pattern.len();
        } else {
            i += 1;
        }
    }
    count
}

fn count_cf_e0(payload: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i + 3 <= payload.len() {
        if payload[i] == 0xCF && payload[i + 1] == 0xE0 && payload[i + 2] == 0x00 {
            count += 1;
            i += 3;
        } else {
            i += 1;
        }
    }
    count
}

fn extract_codename(payload: &[u8]) -> Option<String> {
    // Look for the first CF40 marker (9 bytes), then expect 06 <len> <string>
    let mut i = 0;
    while i + 11 <= payload.len() {
        if payload[i] == 0xCF
            && payload[i + 1] == 0x40
            && payload[i + 2] == 0x00
            && payload[i + 3] == 0x00
        {
            // Marker is 9 bytes; next byte should be 0x06 (string wire tag), then length, then string
            if payload[i + 9] == 0x06 {
                let len = payload[i + 10] as usize;
                let start = i + 11;
                let end = start + len;
                if end <= payload.len() {
                    return std::str::from_utf8(&payload[start..end])
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            return None;
        }
        i += 1;
    }
    None
}

fn extract_last_cf_e0_guid(payload: &[u8]) -> Option<String> {
    // Find the LAST CF E0 00 marker and read the 6-byte tail.
    let mut last_pos: Option<usize> = None;
    let mut i = 0;
    while i + 9 <= payload.len() {
        if payload[i] == 0xCF && payload[i + 1] == 0xE0 && payload[i + 2] == 0x00 {
            last_pos = Some(i);
            i += 9;
        } else {
            i += 1;
        }
    }
    let i = last_pos?;
    if i + 9 > payload.len() {
        return None;
    }
    let tail = &payload[i + 3..i + 9];
    let tail_hex: String = tail.iter().map(|b| format!("{b:02X}")).collect();
    Some(format!("E000{tail_hex}"))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path = PathBuf::from("/tmp/hashes_filename.txt");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" => {
                hash_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    println!("fqn\tsize\tcodename\tcf_e0_count\ttriplet_count\thas_transition\tsig_guid");

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
                if !obj.fqn.starts_with("dis.") {
                    continue;
                }
                let codename = extract_codename(&obj.payload).unwrap_or_else(|| "?".to_string());
                let cf_e0_count = count_cf_e0(&obj.payload);
                let triplet_count = count_pattern(&obj.payload, &[0x02, 0x03, 0x03]);
                let has_transition = count_pattern(&obj.payload, TRANSITION_MARKER) > 0;
                let sig = extract_last_cf_e0_guid(&obj.payload).unwrap_or_else(|| "?".to_string());
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    obj.fqn,
                    obj.payload.len(),
                    codename,
                    cf_e0_count,
                    triplet_count,
                    has_transition,
                    sig
                );
            }
        }
    }

    Ok(())
}
