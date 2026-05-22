//! Comprehensive search: do MAPPINGS.md CC hashes match low32 of property record IDs (or 8-byte type IDs)?

use serde_json::Value;
use std::collections::HashMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;

    let targets: Vec<(&str, u32)> = vec![
        ("6F6FAE37 (talent stringRef)", 0x6F6FAE37),
        ("17E2840B (talent abilityRef)", 0x17E2840B),
        ("E4AFDD03 (talent effectField)", 0xE4AFDD03),
        ("0CCD312D (talent unknown)", 0x0CCD312D),
        ("9D4BD719 (talent effectAnchor)", 0x9D4BD719),
        ("964BD719 (talent altAnchor)", 0x964BD719),
        ("D954FB02 (effect block level)", 0xD954FB02),
        ("D954FB05 (effect block header)", 0xD954FB05),
        ("A787EE87 (talent definition)", 0xA787EE87),
        ("5CE87488 (string table block)", 0x5CE87488),
    ];

    let mut by_id_low32: HashMap<u32, Vec<String>> = HashMap::new();
    let mut by_id_hi32: HashMap<u32, Vec<String>> = HashMap::new();
    let mut by_ref_full: HashMap<String, Vec<String>> = HashMap::new();
    for p in &props {
        let id_s = p["id_hex"].as_str().unwrap_or("");
        if let Ok(id) = u64::from_str_radix(id_s, 16) {
            by_id_low32
                .entry(id as u32)
                .or_default()
                .push(id_s.to_string());
            by_id_hi32
                .entry((id >> 32) as u32)
                .or_default()
                .push(id_s.to_string());
        }
        if let Some(rv) = p["ref_value_hex"].as_str() {
            by_ref_full
                .entry(rv.to_string())
                .or_default()
                .push(id_s.to_string());
        }
    }

    println!("Cross-reference of MAPPINGS.md CC hashes against client.gom property records\n");
    for (lbl, tgt) in &targets {
        let lo_match = by_id_low32.get(tgt).cloned().unwrap_or_default();
        let hi_match = by_id_hi32.get(tgt).cloned().unwrap_or_default();
        let full_padded = format!("00000000{:08X}", tgt);
        let ref_match = by_ref_full.get(&full_padded).cloned().unwrap_or_default();
        let full_padded2 = format!("40000000{:08X}", tgt);
        let ref_match2 = by_ref_full.get(&full_padded2).cloned().unwrap_or_default();

        println!("{}", lbl);
        if !lo_match.is_empty() {
            println!(
                "  -> matches prop id LOW32 of: {}",
                lo_match
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !hi_match.is_empty() {
            println!(
                "  -> matches prop id HI32 of: {}",
                hi_match
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !ref_match.is_empty() {
            println!(
                "  -> matches as ref_value '00000000{:08X}' from: {}",
                tgt,
                ref_match
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !ref_match2.is_empty() {
            println!(
                "  -> matches as ref_value '40000000{:08X}' from: {}",
                tgt,
                ref_match2
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if lo_match.is_empty()
            && hi_match.is_empty()
            && ref_match.is_empty()
            && ref_match2.is_empty()
        {
            println!("  -> NO MATCH anywhere in property records");
        }
        println!();
    }

    Ok(())
}
