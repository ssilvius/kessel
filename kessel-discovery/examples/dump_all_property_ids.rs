//! Dump all 10006 property record ids in various interpretations.

use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;

    let mut high32_freq: BTreeMap<u32, usize> = BTreeMap::new();
    let mut all = Vec::new();

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
                let id_lo = u32::from_le_bytes(body[0x04..0x08].try_into()?);
                let id_hi = u32::from_le_bytes(body[0x08..0x0C].try_into()?);
                *high32_freq.entry(id_hi).or_insert(0) += 1;
                all.push((pos, id_hi, id_lo));
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    println!("=== id high-32 frequency (top 20) ===");
    let mut v: Vec<(u32, usize)> = high32_freq.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (h, n) in v.iter().take(40) {
        println!("  high32 = {:08X}  count = {}", h, n);
    }

    println!("\n=== first 30 prop ids ===");
    for (off, h, l) in all.iter().take(30) {
        println!("  @{:#08x}  full = {:08X}{:08X}", off, h, l);
    }

    Ok(())
}
