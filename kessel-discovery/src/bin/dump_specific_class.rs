//! Dump the talent class record at @0x0ca968.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let targets = vec![
        ("tal class", 0x0ca968usize, 120),
        ("abl class", 0x0b0a30, 456),
        ("itm class", 0x0afb30, 560),
        ("qst class", 0x0b24b8, 648),
    ];

    for (lbl, off, sz) in &targets {
        println!("\n===== {} @ {:#x} size={} =====", lbl, off, sz);
        let body = &data[*off..*off + *sz];
        for (i, b) in body.iter().enumerate() {
            if i % 16 == 0 {
                print!("\n    {:03x}: ", i);
            }
            print!("{:02x} ", b);
        }
        println!();
        // Strings
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
        println!("    strings: {:?}", strs);
    }
    Ok(())
}
