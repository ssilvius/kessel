//! Walk client.gom and extract strings from each record's body.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    assert_eq!(&data[0..4], b"DBLB");
    let mut pos: usize = 8;
    let mut idx = 0;

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

        // Extract ASCII strings (length >= 3) using crude null-split
        let mut strings = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        for &b in body {
            if (32..127).contains(&b) {
                cur.push(b);
            } else {
                if cur.len() >= 3 {
                    if let Ok(s) = std::str::from_utf8(&cur) {
                        strings.push(s.to_string());
                    }
                }
                cur.clear();
            }
        }
        if cur.len() >= 3 {
            if let Ok(s) = std::str::from_utf8(&cur) {
                strings.push(s.to_string());
            }
        }

        // Read field header for context
        let id_a = u32::from_le_bytes(body[4..8].try_into()?);
        let id_b = u32::from_le_bytes(body[8..12].try_into()?);
        let f0c = u32::from_le_bytes(body[12..16].try_into()?);
        let f10 = u16::from_le_bytes(body[16..18].try_into()?);
        let c1 = u16::from_le_bytes(body[18..20].try_into()?);

        if !strings.is_empty() {
            let joined = strings.join(" | ");
            println!(
                "REC {:5} @ {:#08x} sz={:4} id={:08X}{:08X} f0c={:08x} f10={:04x} c1={:#06x} : {}",
                idx, pos, size, id_a, id_b, f0c, f10, c1, joined
            );
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }
    println!("Total records: {}", idx);
    Ok(())
}
