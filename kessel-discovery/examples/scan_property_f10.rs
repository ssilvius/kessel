//! Examine f10 byte distribution in property records.

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;

    // (f10, count_field, first_tail_byte) -> count
    let mut f10_dist: BTreeMap<u16, usize> = BTreeMap::new();
    let mut combo: BTreeMap<(u16, u16, u8), usize> = BTreeMap::new();

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
                let f10 = u16::from_le_bytes(body[0x10..0x12].try_into()?);
                let count = u16::from_le_bytes(body[0x18..0x1A].try_into()?);
                let tail = body[0x20];
                *f10_dist.entry(f10).or_insert(0) += 1;
                *combo.entry((f10, count, tail)).or_insert(0) += 1;
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    println!("=== f10 distribution ===");
    for (f, n) in &f10_dist {
        println!("  f10={:#06x} -> {}", f, n);
    }
    println!("\n=== (f10, count_field, first_tail_byte) ===");
    for ((f, c, t), n) in &combo {
        println!(
            "  f10={:#06x} count={:#06x} tail0={:#04x} -> {}",
            f, c, t, n
        );
    }

    Ok(())
}
