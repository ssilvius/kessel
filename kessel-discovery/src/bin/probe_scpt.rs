//! Reverse-engineer the SCPT compiled-script format from
//! /resources/systemgenerated/compilednative/*.
//!
//! Goals:
//! - locate every SCPT file across all .tor archives by magic
//! - dump header bytes, identify common header layout
//! - detect body encoding (zstd/zlib/lz4/xor/plaintext)
//! - print string-table candidates to identify GSF scripts

use anyhow::Result;
use flate2::read::ZlibDecoder;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

const SCPT_MAGIC: [u8; 4] = *b"SCPT";
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const LZ4_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];
const ZLIB_HEADERS: &[[u8; 2]] = &[[0x78, 0x01], [0x78, 0x9C], [0x78, 0xDA], [0x78, 0x5E]];

fn hexdump(bytes: &[u8], max: usize) {
    let limit = bytes.len().min(max);
    for i in (0..limit).step_by(16) {
        let chunk = &bytes[i..(i + 16).min(limit)];
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

fn try_zlib(data: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn try_zstd(data: &[u8]) -> Option<Vec<u8>> {
    zstd::decode_all(data).ok()
}

fn entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut freq = [0u64; 256];
    for &b in data {
        freq[b as usize] += 1;
    }
    let n = data.len() as f64;
    let mut h = 0.0;
    for &c in &freq {
        if c > 0 {
            let p = c as f64 / n;
            h -= p * p.log2();
        }
    }
    h
}

/// Extract ASCII strings of length >= min_len from the buffer.
fn extract_strings(data: &[u8], min_len: usize, max_strings: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for &b in data {
        if (32..127).contains(&b) {
            cur.push(b);
        } else {
            if cur.len() >= min_len {
                if let Ok(s) = std::str::from_utf8(&cur) {
                    out.push(s.to_string());
                    if out.len() >= max_strings {
                        return out;
                    }
                }
            }
            cur.clear();
        }
    }
    if cur.len() >= min_len {
        if let Ok(s) = std::str::from_utf8(&cur) {
            out.push(s.to_string());
        }
    }
    out
}

#[derive(Debug, Clone)]
struct ScptFile {
    archive: String,
    hash: u64,
    path: Option<String>,
    data: Vec<u8>,
}

fn collect_scpt(
    input_dir: &PathBuf,
    hashes: &HashDictionary,
    limit: usize,
) -> Result<Vec<ScptFile>> {
    let tor_files: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut out = Vec::new();
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
            if entry.compressed_size < 16 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < 16 || data[0..4] != SCPT_MAGIC {
                continue;
            }
            let path = hashes.get(entry.filename_hash).cloned();
            out.push(ScptFile {
                archive: tor_path.file_name().unwrap().to_string_lossy().to_string(),
                hash: entry.filename_hash,
                path,
                data,
            });
            if out.len() >= limit {
                break 'outer;
            }
        }
    }
    Ok(out)
}

fn analyze_one(f: &ScptFile, verbose: bool) {
    println!(
        "=== SCPT: archive={} hash=0x{:016X} path={} size={} ===",
        f.archive,
        f.hash,
        f.path.as_deref().unwrap_or("(unknown)"),
        f.data.len()
    );

    if f.data.len() < 48 {
        println!("  TOO SMALL");
        return;
    }

    // Header guesses
    let bytes = &f.data;
    let version_word = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let version_a = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let version_b = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
    let field_08 = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let field_0c = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let field_10 = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    let field_18 = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let field_1c = u32::from_le_bytes(bytes[28..32].try_into().unwrap());

    println!("  HDR:  version_word=0x{version_word:08X} ({version_a},{version_b})  field_08=0x{field_08:08X}  field_0c=0x{field_0c:08X}");
    println!("        field_10=0x{field_10:016X} ({field_10})  field_18=0x{field_18:08X}  field_1c=0x{field_1c:08X}");

    // Hexdump head
    hexdump(bytes, 96);

    // Find magic offsets for known compressed formats inside the body
    let mut zstd_off = None;
    let mut zlib_off = None;
    let mut lz4_off = None;
    for k in 0..(bytes.len().saturating_sub(4)).min(2048) {
        if zstd_off.is_none() && bytes[k..k + 4] == ZSTD_MAGIC {
            zstd_off = Some(k);
        }
        if lz4_off.is_none() && bytes[k..k + 4] == LZ4_MAGIC {
            lz4_off = Some(k);
        }
        if zlib_off.is_none() && ZLIB_HEADERS.iter().any(|h| &bytes[k..k + 2] == h) {
            // also check this is plausible zlib start: 0x78 followed by valid second byte
            zlib_off = Some(k);
        }
    }

    if let Some(o) = zstd_off {
        println!("  ZSTD magic at offset 0x{o:X}");
        if let Some(decompressed) = try_zstd(&bytes[o..]) {
            println!(
                "    decompressed: {} bytes (entropy {:.2})",
                decompressed.len(),
                entropy(&decompressed)
            );
            if verbose {
                hexdump(&decompressed, 256);
            }
            for s in extract_strings(&decompressed, 6, 20) {
                println!("    str: {s}");
            }
            return;
        }
    }
    if let Some(o) = zlib_off {
        println!("  ZLIB header at offset 0x{o:X}");
        if let Some(decompressed) = try_zlib(&bytes[o..]) {
            println!(
                "    decompressed: {} bytes (entropy {:.2})",
                decompressed.len(),
                entropy(&decompressed)
            );
            if verbose {
                hexdump(&decompressed, 256);
            }
            for s in extract_strings(&decompressed, 6, 20) {
                println!("    str: {s}");
            }
            return;
        }
    }
    if let Some(o) = lz4_off {
        println!("  LZ4 magic at offset 0x{o:X}");
    }

    // Try zstd/zlib starting at various offsets from header
    for try_off in [0x18, 0x1C, 0x20, 0x21, 0x24, 0x28, 0x2A] {
        if let Some(decompressed) = try_zstd(&bytes[try_off..]) {
            println!(
                "  ZSTD raw stream at offset 0x{try_off:X}: decompressed {} bytes (entropy {:.2})",
                decompressed.len(),
                entropy(&decompressed)
            );
            for s in extract_strings(&decompressed, 6, 20) {
                println!("    str: {s}");
            }
            return;
        }
        if let Some(decompressed) = try_zlib(&bytes[try_off..]) {
            println!(
                "  ZLIB raw stream at offset 0x{try_off:X}: decompressed {} bytes (entropy {:.2})",
                decompressed.len(),
                entropy(&decompressed)
            );
            for s in extract_strings(&decompressed, 6, 20) {
                println!("    str: {s}");
            }
            return;
        }
    }

    // Entropy of body
    let body_ent = entropy(&bytes[32..]);
    println!("  no known compression magic found; body entropy={body_ent:.2}");
    let strs = extract_strings(bytes, 6, 20);
    if !strs.is_empty() {
        println!("  raw strings in file:");
        for s in &strs {
            println!("    str: {s}");
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut limit = 10usize;
    let mut needle: Option<String> = None;
    let mut summary = false;
    let mut verbose = false;
    let mut hash_filter: Option<u64> = None;

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
                limit = args[i + 1].parse().unwrap_or(10);
                i += 2;
            }
            "--needle" => {
                needle = Some(args[i + 1].clone());
                i += 2;
            }
            "--summary" => {
                summary = true;
                i += 1;
            }
            "--verbose" => {
                verbose = true;
                i += 1;
            }
            "--hash" => {
                let raw = &args[i + 1];
                hash_filter = if let Some(hex) = raw.strip_prefix("0x") {
                    Some(u64::from_str_radix(hex, 16)?)
                } else {
                    Some(raw.parse()?)
                };
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let all_files = collect_scpt(&input_dir, &hashes, usize::MAX)?;
    eprintln!("Found {} SCPT files total", all_files.len());

    let filtered: Vec<&ScptFile> = if let Some(h) = hash_filter {
        all_files.iter().filter(|f| f.hash == h).collect()
    } else if let Some(ref n) = needle {
        // search body strings for needle (decompress first if possible)
        let mut hits = Vec::new();
        let needle_lc = n.to_lowercase();
        for f in &all_files {
            // try a quick zstd / zlib at common offsets
            let mut decoded: Option<Vec<u8>> = None;
            for try_off in [0x18usize, 0x1C, 0x20, 0x21, 0x24, 0x28, 0x2A] {
                if f.data.len() < try_off + 4 {
                    break;
                }
                if let Some(d) = try_zstd(&f.data[try_off..]) {
                    decoded = Some(d);
                    break;
                }
                if let Some(d) = try_zlib(&f.data[try_off..]) {
                    decoded = Some(d);
                    break;
                }
            }
            if decoded.is_none() {
                // also try at any zstd/zlib magic
                for k in 0..f.data.len().saturating_sub(4) {
                    if f.data[k..k + 4] == ZSTD_MAGIC {
                        if let Some(d) = try_zstd(&f.data[k..]) {
                            decoded = Some(d);
                            break;
                        }
                    }
                    if ZLIB_HEADERS.iter().any(|h| f.data[k..k + 2] == *h) {
                        if let Some(d) = try_zlib(&f.data[k..]) {
                            decoded = Some(d);
                            break;
                        }
                    }
                }
            }
            let buf = decoded.unwrap_or_else(|| f.data.clone());
            let s = String::from_utf8_lossy(&buf).to_lowercase();
            if s.contains(&needle_lc) {
                hits.push(f);
            }
        }
        eprintln!("needle '{n}' matched {} files", hits.len());
        hits
    } else {
        all_files.iter().take(limit).collect()
    };

    if summary {
        let mut by_arch: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_version: BTreeMap<u32, usize> = BTreeMap::new();
        let mut sizes: Vec<usize> = Vec::new();
        for f in &all_files {
            *by_arch.entry(f.archive.clone()).or_insert(0) += 1;
            let v = u32::from_le_bytes(f.data[4..8].try_into().unwrap());
            *by_version.entry(v).or_insert(0) += 1;
            sizes.push(f.data.len());
        }
        sizes.sort();
        println!("--- summary ---");
        println!("total SCPT files: {}", all_files.len());
        println!("by archive:");
        for (a, n) in &by_arch {
            println!("  {a:60} {n}");
        }
        println!("by version word @offset 4 (u32 LE):");
        for (v, n) in &by_version {
            println!("  0x{v:08X}  {n}");
        }
        if !sizes.is_empty() {
            println!(
                "size min={} median={} max={}",
                sizes[0],
                sizes[sizes.len() / 2],
                sizes[sizes.len() - 1]
            );
        }
    }

    for f in filtered {
        analyze_one(f, verbose);
        println!();
    }

    Ok(())
}
