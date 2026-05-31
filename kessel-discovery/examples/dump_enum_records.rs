//! Dump raw bytes of a few enum records (first_str_off == 0x1e) to confirm they store names inline.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;
    let mut printed = 0;

    while pos + 4 <= data.len() && printed < 4 {
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
            if fso == 0x1e {
                println!("\n@ {:#08x} (size={}):", pos, size);
                for (i, b) in body.iter().enumerate() {
                    if i % 16 == 0 {
                        print!("\n    {:02x}: ", i);
                    }
                    print!("{:02x} ", b);
                }
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
                printed += 1;
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    Ok(())
}
