//! Dump raw bytes of a sample of class records (first_str_off == 0x34).

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;

    // Collect first ~6 class records of varying sizes
    let mut count = 0;
    let mut bigcount = 0;

    while pos + 4 <= data.len() && count < 6 {
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
            if fso == 0x34 {
                // Print first 6 small ones
                if size <= 128 && count < 3 {
                    print_record(pos, body);
                    count += 1;
                }
                // Print first big one
                if size > 128 && bigcount < 3 {
                    print_record(pos, body);
                    bigcount += 1;
                    count += 1;
                }
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
    }

    Ok(())
}

fn print_record(off: usize, body: &[u8]) {
    println!("\n@ {:#08x} (size={}):", off, body.len());
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
}
