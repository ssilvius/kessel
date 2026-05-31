//! Scan ALL property records (first_str_off==0x20) and count the type-code byte at offset 0x20.

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;
    let mut idx = 0;

    let mut tail_first: BTreeMap<u8, usize> = BTreeMap::new();
    let mut size_by_first: BTreeMap<(u8, usize), usize> = BTreeMap::new();
    let mut total = 0usize;

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
            if fso == 0x20 {
                let t = body[0x20];
                *tail_first.entry(t).or_insert(0) += 1;
                *size_by_first.entry((t, size)).or_insert(0) += 1;
                total += 1;
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }

    println!("Total property records scanned: {}", total);
    println!();
    println!("=== First tail byte distribution ===");
    for (b, n) in &tail_first {
        println!("  0x{:02x} -> {:5}", b, n);
    }
    println!();
    println!("=== (first tail byte, total record size) -> count ===");
    for ((b, sz), n) in &size_by_first {
        println!("  0x{:02x} sz={:3} -> {}", b, sz, n);
    }

    Ok(())
}
