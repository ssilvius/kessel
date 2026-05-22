//! Dump talent class record and compute prop count.

use std::fs;

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;

    // tal class
    let off = 0x0ca968usize;
    let sz = 120usize;
    let body = &data[off..off + sz];
    println!("===== tal class @ {:#x} size={} =====", off, sz);
    for (i, b) in body.iter().enumerate() {
        if i % 16 == 0 {
            print!("\n    {:03x}: ", i);
        }
        print!("{:02x} ", b);
    }
    println!();
    println!(
        "  size: {}",
        u32::from_le_bytes(body[0..4].try_into().unwrap())
    );
    println!(
        "  id: {:016X}",
        u64::from_le_bytes(body[4..12].try_into().unwrap())
    );
    println!(
        "  f0c: {:08X}",
        u32::from_le_bytes(body[12..16].try_into().unwrap())
    );
    println!(
        "  f10: {:04X}",
        u16::from_le_bytes(body[16..18].try_into().unwrap())
    );
    println!(
        "  fso: {:04X}",
        u16::from_le_bytes(body[18..20].try_into().unwrap())
    );
    println!(
        "  off_b: {:04X}",
        u16::from_le_bytes(body[20..22].try_into().unwrap())
    );
    println!(
        "  off_c: {:04X}",
        u16::from_le_bytes(body[22..24].try_into().unwrap())
    );

    // The 8-byte fields at 0x18, 0x20 might be GUIDs (16 bytes total = 2 guids = parent class refs)
    println!("  bytes 0x18..0x28 (16 bytes, possibly 2 8-byte GUIDs):");
    println!(
        "    {:016X}",
        u64::from_le_bytes(body[0x18..0x20].try_into().unwrap())
    );
    println!(
        "    {:016X}",
        u64::from_le_bytes(body[0x20..0x28].try_into().unwrap())
    );
    println!(
        "  u16 at 0x28: {:04X}",
        u16::from_le_bytes(body[0x28..0x2a].try_into().unwrap())
    );
    println!(
        "  u16 at 0x2a: {:04X}",
        u16::from_le_bytes(body[0x2a..0x2c].try_into().unwrap())
    );
    println!(
        "  u16 at 0x2c: {:04X}",
        u16::from_le_bytes(body[0x2c..0x2e].try_into().unwrap())
    );
    println!(
        "  u16 at 0x2e: {:04X}",
        u16::from_le_bytes(body[0x2e..0x30].try_into().unwrap())
    );
    println!(
        "  u32 at 0x30: {:08X}",
        u32::from_le_bytes(body[0x30..0x34].try_into().unwrap())
    );

    // Property GUIDs from offset 0x38 onward, 8 bytes each
    println!("\n  Property GUIDs (starting at 0x38):");
    let mut i = 0x38;
    while i + 8 <= sz {
        let gid = u64::from_le_bytes(body[i..i + 8].try_into().unwrap());
        println!("    @{:#x} = {:016X}", i, gid);
        i += 8;
    }
    println!(
        "  (used {} bytes for {} property GUIDs)",
        i - 0x38,
        (i - 0x38) / 8
    );

    Ok(())
}
