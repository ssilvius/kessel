//! Test whether the well-known GOM type IDs from MAPPINGS.md match class record ids in client.gom.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;
    let mut classes: Vec<(usize, u64, u16, usize)> = Vec::new();

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
                let id = u64::from_le_bytes(body[0x04..0x0C].try_into()?);
                let count = u16::from_le_bytes(body[0x18..0x1A].try_into()?);
                classes.push((pos, id, count, size));
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    println!("Total class records: {}", classes.len());

    // GOM object header type IDs (from MAPPINGS.md, bytes 16-19, little-endian)
    let targets: Vec<(&str, u32)> = vec![
        ("tal.*", 0xd954fb01), // 01fb54d9 read as bytes, so LE
        ("tal.* alt", 0x01fb54d9),
        ("abl.*", 0x0283f4d2),
        ("abl.* alt", 0xd2f48302),
        ("qst.*", 0x2adec3d2),
        ("qst.* alt", 0xd2c3de2a),
        ("itm.*", 0x011acd0e),
        ("itm.* alt", 0x0ecd1a01),
        ("npc.*", 0x0078e1bd),
        ("npc.* alt", 0xbde17800),
        ("mpn.*", 0xf9e467c7),
        ("mpn.* alt", 0xc767e4f9),
        ("cdx.*", 0x257639ec),
        ("cdx.* alt", 0xec397625),
        ("ach.*", 0x3ac53ea0),
        ("ach.* alt", 0xa03ec53a),
        ("schem.*", 0xdfa8408a),
        ("schem.* alt", 0x8a40a8df),
    ];

    for (lbl, tgt) in &targets {
        let mut hits = Vec::new();
        for (off, id, count, sz) in &classes {
            let lo = *id as u32;
            let hi = (*id >> 32) as u32;
            if lo == *tgt || hi == *tgt {
                hits.push((*off, *id, *count, *sz, lo == *tgt));
            }
        }
        if !hits.is_empty() {
            println!("\n{}  (target u32={:08X}):", lbl, tgt);
            for (off, id, cnt, sz, low) in &hits {
                println!(
                    "  @{:#08x} id={:016X} count={} sz={} (matched {})",
                    off,
                    id,
                    cnt,
                    sz,
                    if *low { "low" } else { "high" }
                );
            }
        } else {
            println!("\n{}  (target u32={:08X}): NO MATCH", lbl, tgt);
        }
    }
    Ok(())
}
