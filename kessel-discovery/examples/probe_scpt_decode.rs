//! Try various decoding transformations on the SCPT body.

use anyhow::Result;
use flate2::read::ZlibDecoder;
use kessel::myp::Archive;
use std::io::Read;
use std::path::PathBuf;

const SCPT_MAGIC: [u8; 4] = *b"SCPT";

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

fn hexdump(label: &str, bytes: &[u8], max: usize) {
    let limit = bytes.len().min(max);
    println!("  {label}:");
    for i in (0..limit).step_by(32) {
        let chunk = &bytes[i..(i + 32).min(limit)];
        let hex: String = chunk
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let asc: String = chunk
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("    {i:04X}  {hex}  {asc}");
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut limit = 3usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-n" => {
                limit = args[i + 1].parse().unwrap_or(3);
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
            if scpts.len() >= limit {
                break 'outer;
            }
        }
    }
    eprintln!("collected {} SCPT samples", scpts.len());

    let body_start = 0x25usize;

    for (h, d) in &scpts {
        println!(
            "\n=== file hash=0x{h:016X} total={} body_len={} ===",
            d.len(),
            d.len() - body_start
        );
        let body = &d[body_start..];
        hexdump("raw body", body, 96);

        // Try 1: body[0] as XOR mask, applied to body[1..]
        let mask = body[0];
        let xored: Vec<u8> = body[1..].iter().map(|b| b ^ mask).collect();
        println!(
            "  XOR with body[0]=0x{mask:02X} -> entropy {:.3}",
            entropy(&xored)
        );
        hexdump("body[1..] ^ body[0]", &xored, 96);

        // Try 2: body[0] as initial XOR mask, key is body[0..1] but increment each byte
        let mask = body[0];
        let inc_xor: Vec<u8> = body[1..]
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ (mask.wrapping_add(i as u8)))
            .collect();
        println!("  XOR with body[0] + i -> entropy {:.3}", entropy(&inc_xor));
        hexdump("body[1..] ^ (body[0]+i)", &inc_xor, 96);

        // Try 3: subtract previous byte from each byte (delta encoding)
        let mut prev = body[0];
        let delta: Vec<u8> = body[1..]
            .iter()
            .map(|&b| {
                let v = b.wrapping_sub(prev);
                prev = b;
                v
            })
            .collect();
        println!("  delta (b[i]-b[i-1]) -> entropy {:.3}", entropy(&delta));
        hexdump("delta", &delta, 96);

        // Try 4: rolling XOR (b[i] ^ b[i-1])
        let mut prev = 0u8;
        let rxor: Vec<u8> = body
            .iter()
            .map(|&b| {
                let v = b ^ prev;
                prev = b;
                v
            })
            .collect();
        println!(
            "  rolling XOR (b[i]^b[i-1]) -> entropy {:.3}",
            entropy(&rxor)
        );
        hexdump("rolling XOR", &rxor, 96);

        // Try 5: try zstd on the body as-is
        if let Ok(out) = zstd::decode_all(body) {
            println!(
                "  ZSTD-raw OK: {} bytes (entropy {:.3})",
                out.len(),
                entropy(&out)
            );
            hexdump("zstd-raw", &out, 128);
        }
        // Try 6: try zlib raw
        let mut dec = ZlibDecoder::new(body);
        let mut out = Vec::new();
        if dec.read_to_end(&mut out).is_ok() && !out.is_empty() {
            println!(
                "  ZLIB OK: {} bytes (entropy {:.3})",
                out.len(),
                entropy(&out)
            );
            hexdump("zlib", &out, 128);
        }

        // Try 7: skip body[0] then try zstd, zlib, raw deflate
        if body.len() > 1 {
            let body1 = &body[1..];
            if let Ok(out) = zstd::decode_all(body1) {
                println!(
                    "  ZSTD skip body[0]: {} bytes (entropy {:.3})",
                    out.len(),
                    entropy(&out)
                );
                hexdump("zstd-skip", &out, 128);
            }
            let mut dec = ZlibDecoder::new(body1);
            let mut out = Vec::new();
            if dec.read_to_end(&mut out).is_ok() && !out.is_empty() {
                println!(
                    "  ZLIB skip body[0]: {} bytes (entropy {:.3})",
                    out.len(),
                    entropy(&out)
                );
                hexdump("zlib-skip", &out, 128);
            }
            let mut dec = flate2::read::DeflateDecoder::new(body1);
            let mut out = Vec::new();
            if dec.read_to_end(&mut out).is_ok() && !out.is_empty() {
                println!(
                    "  DEFLATE skip body[0]: {} bytes (entropy {:.3})",
                    out.len(),
                    entropy(&out)
                );
                hexdump("deflate-skip", &out, 128);
            }
        }

        // Try 8: subtract body[i] from a running counter
        // Try 9: XOR with PRNG seeded by GUID
        let guid = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
        let mut state = guid;
        let prng: Vec<u8> = body
            .iter()
            .map(|b| {
                // simple LCG
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (b ^ (state >> 33) as u8)
            })
            .collect();
        println!("  LCG-PRNG XOR -> entropy {:.3}", entropy(&prng));

        // Try 10: rotate each byte by N
        for rot in 1..8 {
            let rotated: Vec<u8> = body.iter().map(|b| b.rotate_left(rot)).collect();
            // check if first 4 bytes match a known magic after rotation
            let first4 = u32::from_le_bytes(rotated[0..4].try_into().unwrap());
            if first4 == 0xF1E0_F1E0 || first4 == 0x504D_4143 || first4 == 0x4D414350 {
                println!("  ROL{rot} produces magic at offset 0!");
            }
        }
    }

    Ok(())
}
