//! Probe SCPT bodies for XOR-key patterns.
//!
//! Hypothesis: body is XOR'd with a key derived from the header (GUID) or
//! a fixed cipher. Look for repeating patterns at known offsets in multiple
//! files (the start of a Pawn/AMX bytecode header should be identical).

use anyhow::Result;
use kessel::myp::Archive;
use std::path::PathBuf;

const SCPT_MAGIC: [u8; 4] = *b"SCPT";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut max_files = 20usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-n" => {
                max_files = args[i + 1].parse().unwrap_or(20);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut scpts: Vec<(u64, Vec<u8>)> = Vec::new();
    'outer: for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if entry.compressed_size < 64 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < 64 || data[0..4] != SCPT_MAGIC {
                continue;
            }
            scpts.push((entry.filename_hash, data));
            if scpts.len() >= max_files {
                break 'outer;
            }
        }
    }
    eprintln!("collected {} SCPT samples", scpts.len());

    let body_start = 0x25usize;

    // Print first 32 bytes of body in each file
    println!("\n== first 32 body bytes per file (offset 0x25 onward) ==");
    for (h, d) in &scpts {
        let end = (body_start + 32).min(d.len());
        let chunk = &d[body_start..end];
        let hex: String = chunk
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  hash=0x{h:016X}  body[0..32]={hex}");
    }

    // XOR every byte of body with the GUID (8 bytes from header offset 0x10),
    // rotated cyclically.
    println!("\n== XOR body with GUID @0x10 cyclically -- first 64 bytes ==");
    for (h, d) in scpts.iter().take(5) {
        let key = &d[0x10..0x18];
        let body = &d[body_start..(body_start + 64).min(d.len())];
        let out: Vec<u8> = body
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        let hex: String = out
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let asc: String = out
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  hash=0x{h:016X}");
        println!("    hex: {hex}");
        println!("    asc: {asc}");
    }

    // XOR body with whole 37-byte header cyclically
    println!("\n== XOR body with full 0x25-byte header cyclically -- first 64 bytes ==");
    for (h, d) in scpts.iter().take(5) {
        let key = &d[0..0x25];
        let body = &d[body_start..(body_start + 64).min(d.len())];
        let out: Vec<u8> = body
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        let hex: String = out
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let asc: String = out
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  hash=0x{h:016X}");
        println!("    hex: {hex}");
        println!("    asc: {asc}");
    }

    // Look at byte-frequency in body across all samples
    let mut freq = [0u64; 256];
    let mut total = 0u64;
    for (_, d) in &scpts {
        if d.len() <= body_start {
            continue;
        }
        for &b in &d[body_start..] {
            freq[b as usize] += 1;
            total += 1;
        }
    }
    let mut sorted: Vec<(usize, u64)> = freq.iter().enumerate().map(|(i, &c)| (i, c)).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n== body byte frequency (top 16) total bytes={total} ==");
    for (i, (b, c)) in sorted.iter().take(16).enumerate() {
        let pct = 100.0 * (*c as f64) / (total as f64);
        println!("  {i:2}. byte 0x{b:02X}  count={c}  {pct:.3}%");
    }

    // Diff body[0] across files -- if first opcode is constant, the XOR key's
    // first byte would equal body[0] XOR opcode.
    println!("\n== distribution of body[0..4] across files ==");
    let mut head_counts = std::collections::BTreeMap::new();
    for (_, d) in &scpts {
        let key = u32::from_le_bytes(d[body_start..body_start + 4].try_into().unwrap());
        *head_counts.entry(key).or_insert(0u32) += 1;
    }
    for (k, c) in &head_counts {
        println!("  0x{k:08X}  {c} files");
    }

    Ok(())
}
