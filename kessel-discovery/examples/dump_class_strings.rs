//! Dump every string found within class records (fso=0x34).

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;

    let mut len_dist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut total_class = 0;
    let mut with_string = 0;
    let mut samples_with: Vec<(usize, usize, Vec<String>)> = Vec::new();

    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        if size == 0 {
            pos += 1;
            continue;
        }
        if size < 16 || pos + size > data.len() {
            break;
        }
        let body = &data[pos..pos + size];

        if body.len() >= 0x21 {
            let fso = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            if fso == 0x34 {
                total_class += 1;
                // Extract strings within body
                let mut cur: Vec<u8> = Vec::new();
                let mut strs = Vec::new();
                for &b in body {
                    if (32..127).contains(&b) {
                        cur.push(b);
                    } else {
                        if cur.len() >= 3 {
                            strs.push(String::from_utf8_lossy(&cur).to_string());
                        }
                        cur.clear();
                    }
                }
                if cur.len() >= 3 {
                    strs.push(String::from_utf8_lossy(&cur).to_string());
                }
                if !strs.is_empty() {
                    with_string += 1;
                    if samples_with.len() < 10 {
                        samples_with.push((pos, size, strs.clone()));
                    }
                }
                *len_dist.entry(strs.len()).or_insert(0) += 1;
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    println!("Total class records: {}", total_class);
    println!("With strings: {}", with_string);
    println!("\nStrings-per-class distribution:");
    for (n, c) in &len_dist {
        println!("  {} strings -> {} records", n, c);
    }
    println!("\nSamples with strings:");
    for (off, sz, strs) in &samples_with {
        println!("  @{:#08x} sz={} strs={:?}", off, sz, strs);
    }

    Ok(())
}
