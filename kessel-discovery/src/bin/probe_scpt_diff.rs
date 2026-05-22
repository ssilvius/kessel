//! Diff the bodies of two SCPT files byte-by-byte to identify the
//! common-prefix region vs the per-script region.

use anyhow::Result;
use kessel::myp::Archive;
use std::path::PathBuf;

const SCPT_MAGIC: [u8; 4] = *b"SCPT";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut max_files = 50usize;
    let mut dump_len = 256usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-n" => {
                max_files = args[i + 1].parse().unwrap_or(50);
                i += 2;
            }
            "-d" => {
                dump_len = args[i + 1].parse().unwrap_or(256);
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
            if entry.compressed_size < 200 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < 200 || data[0..4] != SCPT_MAGIC {
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

    // For each byte offset within the body, count how many files have the
    // same byte value. The most common value at each offset gives us a
    // "consensus" body. Then dump regions where >50% of files agree.
    let max_len = scpts
        .iter()
        .map(|(_, d)| d.len() - body_start)
        .min()
        .unwrap_or(0);
    let dump_len = dump_len.min(max_len);

    let mut consensus = vec![0u8; dump_len];
    let mut consensus_freq = vec![0u32; dump_len];
    let n = scpts.len() as u32;
    for off in 0..dump_len {
        let mut counts = std::collections::HashMap::new();
        for (_, d) in &scpts {
            let b = d[body_start + off];
            *counts.entry(b).or_insert(0u32) += 1;
        }
        let (b, c) = counts.into_iter().max_by_key(|(_, c)| *c).unwrap();
        consensus[off] = b;
        consensus_freq[off] = c;
    }

    println!("== consensus body[0..{dump_len}] across {n} files (* = >50% agreement) ==");
    for chunk_start in (0..dump_len).step_by(16) {
        let chunk_end = (chunk_start + 16).min(dump_len);
        let hex: String = (chunk_start..chunk_end)
            .map(|i| {
                let star = if consensus_freq[i] * 2 > n { "*" } else { " " };
                format!("{:02X}{}", consensus[i], star)
            })
            .collect::<Vec<_>>()
            .join("");
        let asc: String = (chunk_start..chunk_end)
            .map(|i| {
                if consensus_freq[i] * 2 > n {
                    let b = consensus[i];
                    if (32..127).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                } else {
                    ' '
                }
            })
            .collect();
        let pct: String = (chunk_start..chunk_end)
            .map(|i| format!("{:>3}", 100 * consensus_freq[i] / n))
            .collect::<Vec<_>>()
            .join(",");
        println!("  body[0x{chunk_start:04X}..0x{chunk_end:04X}]  hex: {hex}   asc: {asc:<16}  pct:{pct}");
    }

    // Print actual bodies of first 5 files for comparison
    println!("\n== individual body[0..{dump_len}] for first 5 files ==");
    for (h, d) in scpts.iter().take(5) {
        let end = (body_start + dump_len).min(d.len());
        let chunk = &d[body_start..end];
        println!("\n  hash=0x{h:016X}  body_len={}", d.len() - body_start);
        for s in (0..chunk.len()).step_by(32) {
            let e = (s + 32).min(chunk.len());
            let hex: String = chunk[s..e]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let asc: String = chunk[s..e]
                .iter()
                .map(|&b| {
                    if (32..127).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            println!("    {s:04X}  {hex}  {asc}");
        }
    }
    Ok(())
}
