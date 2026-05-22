//! Dump raw bytes of a sample of property records (first_str_off == 0x20)
//! with multiple sizes so we can reverse the typed tail.

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;

    // Collect first N records per size bucket for size in {33, 34, 35, 36, 37, 41, 42, 43, 44, 45, 51, 53, 63}
    let want: Vec<usize> = vec![
        33, 34, 35, 36, 37, 39, 41, 42, 43, 44, 45, 46, 47, 51, 53, 55, 57, 61, 63,
    ];
    let mut collected: BTreeMap<usize, Vec<(usize, Vec<u8>)>> = BTreeMap::new();

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

        if body.len() >= 32 {
            let fso = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            if fso == 0x20 && want.contains(&size) {
                let v = collected.entry(size).or_default();
                if v.len() < 4 {
                    v.push((pos, body.to_vec()));
                }
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    for (size, recs) in &collected {
        println!(
            "\n===== Property records of size {} (showing {} samples) =====",
            size,
            recs.len()
        );
        for (off, body) in recs {
            print!("\n@ {:#08x} (size={}): ", off, size);
            for (i, b) in body.iter().enumerate() {
                if i % 16 == 0 {
                    print!("\n    {:02x}: ", i);
                }
                print!("{:02x} ", b);
            }
            // also print readable strings
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
            println!("\n    strings: {:?}", strs);
        }
    }

    Ok(())
}
