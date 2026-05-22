//! Walk client.gom and classify records by first_str_off (the type discriminator).
//! Prints distribution counts and a few samples per size bucket.

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    assert_eq!(&data[0..4], b"DBLB");
    let mut pos: usize = 8;
    let mut idx = 0;

    // (first_str_off, size) -> count
    let mut bucket: BTreeMap<(u16, usize), usize> = BTreeMap::new();
    // first_str_off -> count
    let mut by_fso: BTreeMap<u16, usize> = BTreeMap::new();
    // Per first_str_off: collect first 3 example offsets
    let mut samples: BTreeMap<u16, Vec<(usize, usize)>> = BTreeMap::new();

    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        if size == 0 {
            pos += 1;
            continue;
        }
        if size < 16 || pos + size > data.len() {
            println!("REC {} @ {:#x} bad size {}", idx, pos, size);
            break;
        }
        let body = &data[pos..pos + size];

        if body.len() < 32 {
            // too small, skip
        } else {
            let first_str_off = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            *bucket.entry((first_str_off, size)).or_insert(0) += 1;
            *by_fso.entry(first_str_off).or_insert(0) += 1;
            let s = samples.entry(first_str_off).or_default();
            if s.len() < 3 {
                s.push((pos, size));
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }

    println!("Total records: {}", idx);
    println!();
    println!("=== Counts by first_str_off ===");
    for (fso, n) in &by_fso {
        println!("  first_str_off={:#06x} ({:4}) records", fso, n);
    }
    println!();
    println!("=== Counts by (first_str_off, size) ===");
    for ((fso, sz), n) in &bucket {
        println!("  first_str_off={:#06x} size={:4} -> {}", fso, sz, n);
    }
    println!();
    println!("=== Sample offsets per first_str_off ===");
    for (fso, sl) in &samples {
        let formatted: Vec<String> = sl
            .iter()
            .map(|(p, s)| format!("@{:#08x} sz={}", p, s))
            .collect();
        println!("  fso={:#06x}: {}", fso, formatted.join(", "));
    }

    Ok(())
}
