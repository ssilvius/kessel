//! Cross-reference: properties with type=09/0F vs class record IDs (full and low32).

use serde_json::Value;
use std::collections::HashMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let classes: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-classes.json")?)?;

    let mut class_by_low32: HashMap<String, &Value> = HashMap::new();
    let mut class_by_full: HashMap<String, &Value> = HashMap::new();
    let mut class_by_hi32: HashMap<String, &Value> = HashMap::new();
    for c in &classes {
        if let Some(cid) = c["class_id_hex"].as_str() {
            if cid.len() == 16 {
                let low32 = cid[8..].to_string();
                let hi32 = cid[..8].to_string();
                class_by_low32.insert(low32, c);
                class_by_full.insert(cid.to_string(), c);
                class_by_hi32.insert(hi32, c);
            }
        }
    }

    for tag in &["09", "0F"] {
        let mut total = 0;
        let mut matched_low32 = 0;
        let mut matched_full = 0;
        let mut matched_hi32 = 0;
        let mut samples: Vec<String> = Vec::new();
        for p in &props {
            if p["type_tag"].as_str() != Some(tag) {
                continue;
            }
            let Some(rv) = p["ref_value_hex"].as_str() else {
                continue;
            };
            if rv.len() != 16 {
                continue;
            }
            total += 1;
            let low32 = &rv[8..];
            let hi32 = &rv[..8];
            if class_by_full.contains_key(rv) {
                matched_full += 1;
                if samples.len() < 5 {
                    let c = class_by_full.get(rv).unwrap();
                    samples.push(format!(
                        "prop {} -> class FULL {}",
                        p["id_hex"].as_str().unwrap_or(""),
                        c["class_id_hex"].as_str().unwrap_or("")
                    ));
                }
            } else if class_by_low32.contains_key(low32) {
                matched_low32 += 1;
                if samples.len() < 5 {
                    let c = class_by_low32.get(low32).unwrap();
                    samples.push(format!(
                        "prop {} -> class LOW32 {} (low32 {})",
                        p["id_hex"].as_str().unwrap_or(""),
                        c["class_id_hex"].as_str().unwrap_or(""),
                        low32
                    ));
                }
            } else if class_by_hi32.contains_key(hi32) {
                matched_hi32 += 1;
                if samples.len() < 5 {
                    let c = class_by_hi32.get(hi32).unwrap();
                    samples.push(format!(
                        "prop {} -> class HI32 {} (hi32 {})",
                        p["id_hex"].as_str().unwrap_or(""),
                        c["class_id_hex"].as_str().unwrap_or(""),
                        hi32
                    ));
                }
            }
        }
        println!(
            "Type={}: total={} matched_full={} matched_low32={} matched_hi32={}",
            tag, total, matched_full, matched_low32, matched_hi32
        );
        for s in &samples {
            println!("  {}", s);
        }
    }

    Ok(())
}
