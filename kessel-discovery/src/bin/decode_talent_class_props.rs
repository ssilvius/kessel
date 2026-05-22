//! Decode the property records that the talent class's property_refs point to.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;

    // Index by low32 of id (this is the SHORT form used in class.property_refs minus the 4000 prefix)
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    let mut by_hi32: HashMap<String, &Value> = HashMap::new();
    for p in &props {
        if let Some(id) = p["id_hex"].as_str() {
            by_id.insert(id.to_string(), p);
            if id.len() >= 8 {
                by_hi32.insert(id[..8].to_string(), p);
            }
        }
    }

    // Talent class property refs
    let refs: Vec<&str> = vec![
        "40000013A787EE85",
        "40000041205DB411",
        "4000004C780B5130",
        "4000004C7880CF50",
        "40000040D96EC4A1",
        "40000040D954FB07",
        "40000040D96EC4A2",
        "40000040D954FB09",
    ];

    println!("Decoding talent class (D954FB01) property templates:\n");
    for r in &refs {
        // class.property_refs format is 4000XXXX_YYYYYYYY where YYYY... is the hi32 of a property record
        // Take low32 = YYYY YYYY → match property record where id.hi32 = YYYY YYYY
        let low32 = &r[8..];
        println!("Template ref {} -- inner hi32={}:", r, low32);
        if let Some(p) = by_hi32.get(low32) {
            println!(
                "  -> prop {} (sz={}, type={}, kind={})",
                p["id_hex"].as_str().unwrap_or(""),
                p["size"].as_u64().unwrap_or(0),
                p["type_tag"].as_str().unwrap_or(""),
                p["type_kind"].as_str().unwrap_or("")
            );
            if let Some(rv) = p["ref_value_hex"].as_str() {
                println!("    ref_value: {}", rv);
            }
            if let Some(n) = p["resolved_enum_name"].as_str() {
                println!("    resolved enum: {}", n);
            }
            if let Some(c) = p["resolved_class_id"].as_str() {
                println!("    resolved class: {}", c);
            }
        } else {
            println!("  -> NO MATCH (no property record has hi32={})", low32);
        }
        println!();
    }

    Ok(())
}
