//! Find specific class/property IDs in client.gom from MAPPINGS.md known type IDs.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;

    let targets: Vec<(&str, u64)> = vec![
        // Property record IDs referenced in MAPPINGS.md (talent CF type ids)
        ("Talent definition (CF type id)", 0x40000013A787EE87),
        ("Talent definition -2 from list", 0x40000013A787EE85),
        ("String table block", 0x400000115CE87488),
        ("Effect block with level", 0x40000040D954FB02),
        ("Effect block header", 0x40000040D954FB05),
        ("Effect block 07", 0x40000040D954FB07),
        ("Effect block 09", 0x40000040D954FB09),
        ("Other ref C4A1", 0x40000040D96EC4A1),
        ("Other ref C4A2", 0x40000040D96EC4A2),
    ];

    let mut pos: usize = 8;
    let mut records: Vec<(usize, u64, u16, usize, u16)> = Vec::new();
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

        if body.len() >= 0x14 {
            let id = u64::from_le_bytes(body[0x04..0x0C].try_into()?);
            let fso = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            let count = u16::from_le_bytes(body[0x18..0x1A].try_into()?);
            records.push((pos, id, fso, size, count));
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    for (lbl, tgt) in &targets {
        let hits: Vec<_> = records
            .iter()
            .filter(|(_, id, _, _, _)| id == tgt)
            .collect();
        if hits.is_empty() {
            println!("{:50}  ({:016X}): NO MATCH", lbl, tgt);
        } else {
            for (off, id, fso, sz, cnt) in &hits {
                println!(
                    "{:50}  ({:016X}): @{:#08x} fso={:#x} sz={} count={:#x}",
                    lbl, id, off, fso, sz, cnt
                );
            }
        }
    }

    Ok(())
}
