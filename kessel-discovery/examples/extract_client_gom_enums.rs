//! Extract every enum record (c1 = 0x1E) from client.gom as JSON.

use std::collections::BTreeMap;
use std::fs;

#[derive(serde::Serialize)]
struct EnumRecord {
    index: usize,
    offset_hex: String,
    size: usize,
    id_hex: String,
    f0c_hex: String,
    f10_hex: String,
    members: Vec<String>,
    offsets: Vec<u16>,
}

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    assert_eq!(&data[0..4], b"DBLB");
    let mut pos: usize = 8;
    let mut idx = 0;
    let mut enums: Vec<EnumRecord> = Vec::new();
    let mut type_counts: BTreeMap<u16, usize> = BTreeMap::new();

    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        if size == 0 {
            pos += 1;
            continue;
        }
        if pos + size > data.len() {
            break;
        }
        let body = &data[pos..pos + size];
        if body.len() < 30 {
            // too small to be an enum; just advance
            let next = pos + size;
            let pad = (8 - (next % 8)) % 8;
            pos = next + pad;
            idx += 1;
            continue;
        }
        let id_a = u32::from_le_bytes(body[4..8].try_into()?);
        let id_b = u32::from_le_bytes(body[8..12].try_into()?);
        let f0c = u32::from_le_bytes(body[12..16].try_into()?);
        let f10 = u16::from_le_bytes(body[16..18].try_into()?);
        let c1 = u16::from_le_bytes(body[18..20].try_into()?);
        let _c2 = u16::from_le_bytes(body[20..22].try_into()?);
        let _c3 = u16::from_le_bytes(body[22..24].try_into()?);
        let count = u16::from_le_bytes(body[24..26].try_into()?) as usize;
        let tbl_off = u32::from_le_bytes(body[26..30].try_into()?) as usize;

        if c1 == 0x1E && count > 0 && tbl_off >= 0x1E && tbl_off < size {
            // Strings span [0x1E .. tbl_off]
            let strings_blob = &body[0x1E..tbl_off];
            let mut members = Vec::new();
            for slice in strings_blob.split(|&b| b == 0) {
                if !slice.is_empty() {
                    if let Ok(s) = std::str::from_utf8(slice) {
                        members.push(s.to_string());
                    }
                }
            }
            let mut offsets = Vec::new();
            let mut tpos = tbl_off;
            for _ in 0..count {
                if tpos + 2 <= size {
                    let off = u16::from_le_bytes(body[tpos..tpos + 2].try_into()?);
                    offsets.push(off);
                    tpos += 2;
                }
            }
            if members.len() == count {
                enums.push(EnumRecord {
                    index: idx,
                    offset_hex: format!("{:#08x}", pos),
                    size,
                    id_hex: format!("{:08X}{:08X}", id_a, id_b),
                    f0c_hex: format!("{:08X}", f0c),
                    f10_hex: format!("{:04X}", f10),
                    members,
                    offsets,
                });
            }
        }

        *type_counts.entry(c1).or_default() += 1;

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }

    eprintln!("Total records: {}", idx);
    eprintln!(
        "Enum records (c1=0x1E with consistent count): {}",
        enums.len()
    );
    let json = serde_json::to_string_pretty(&enums)?;
    fs::write("/tmp/client-gom-enums.json", &json)?;
    eprintln!("Wrote /tmp/client-gom-enums.json ({} bytes)", json.len());
    Ok(())
}
