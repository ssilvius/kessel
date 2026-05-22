//! Exhaustive search for MAPPINGS.md hashes anywhere in client.gom raw bytes.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;

    let targets: Vec<(&str, &[u8])> = vec![
        ("6F6FAE37 LE", &[0x37, 0xae, 0x6f, 0x6f]),
        ("6F6FAE37 BE", &[0x6f, 0x6f, 0xae, 0x37]),
        ("17E2840B LE", &[0x0b, 0x84, 0xe2, 0x17]),
        ("17E2840B BE", &[0x17, 0xe2, 0x84, 0x0b]),
        ("E4AFDD03 LE", &[0x03, 0xdd, 0xaf, 0xe4]),
        ("E4AFDD03 BE", &[0xe4, 0xaf, 0xdd, 0x03]),
        ("0CCD312D LE", &[0x2d, 0x31, 0xcd, 0x0c]),
        ("0CCD312D BE", &[0x0c, 0xcd, 0x31, 0x2d]),
        ("9D4BD719 LE", &[0x19, 0xd7, 0x4b, 0x9d]),
        ("9D4BD719 BE", &[0x9d, 0x4b, 0xd7, 0x19]),
        ("964BD719 LE", &[0x19, 0xd7, 0x4b, 0x96]),
        ("964BD719 BE", &[0x96, 0x4b, 0xd7, 0x19]),
    ];

    for (lbl, pat) in &targets {
        let mut hits = Vec::new();
        for i in 0..data.len().saturating_sub(pat.len()) {
            if &data[i..i + pat.len()] == *pat {
                hits.push(i);
                if hits.len() > 20 {
                    break;
                }
            }
        }
        println!(
            "{}: {} hits (first 8): {:?}",
            lbl,
            hits.len(),
            hits.iter()
                .take(8)
                .map(|p| format!("{:#x}", p))
                .collect::<Vec<_>>()
        );
    }

    Ok(())
}
