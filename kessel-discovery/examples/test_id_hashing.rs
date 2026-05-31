//! Test whether the 4-byte CC hashes (from MAPPINGS.md) match a hash of any property record's 8-byte id.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;
    let mut prop_ids: Vec<u64> = Vec::new();

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
                let id = u64::from_le_bytes(body[0x04..0x0C].try_into()?);
                prop_ids.push(id);
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    println!("Loaded {} property record ids", prop_ids.len());

    let targets: Vec<(&str, u32)> = vec![
        ("6F6FAE37", 0x6F6FAE37),
        ("17E2840B", 0x17E2840B),
        ("E4AFDD03", 0xE4AFDD03),
        ("0CCD312D", 0x0CCD312D),
        ("9D4BD719", 0x9D4BD719),
    ];

    // Check: low32 of id, high32 of id, low32 XOR high32, FNV-1a hash, etc.
    for (lbl, tgt) in &targets {
        let mut low_hits = 0usize;
        let mut high_hits = 0usize;
        let mut xor_hits = 0usize;
        for id in &prop_ids {
            let lo = *id as u32;
            let hi = (*id >> 32) as u32;
            if lo == *tgt {
                low_hits += 1;
            }
            if hi == *tgt {
                high_hits += 1;
            }
            if (lo ^ hi) == *tgt {
                xor_hits += 1;
            }
        }
        println!(
            "{}: low_hits={} high_hits={} xor_hits={}",
            lbl, low_hits, high_hits, xor_hits
        );
    }

    // Show the structure of GOM IDs - distribution of high16 bits
    use std::collections::BTreeMap;
    let mut hi16: BTreeMap<u16, usize> = BTreeMap::new();
    for id in &prop_ids {
        let h16 = (*id >> 48) as u16;
        *hi16.entry(h16).or_insert(0) += 1;
    }
    println!("\nProp id high-16 distribution (top 10):");
    let mut v: Vec<_> = hi16.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (h, n) in v.iter().take(15) {
        println!("  high16 = {:04X}  count = {}", h, n);
    }

    Ok(())
}
