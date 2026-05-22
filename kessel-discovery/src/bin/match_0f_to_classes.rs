//! Verify: type=0F ref values' low32 matches a class's class_type_hi32.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let classes: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-classes.json")?)?;

    let mut classes_by_hi32: HashMap<String, &Value> = HashMap::new();
    for c in &classes {
        if let Some(h) = c["class_type_hi32"].as_str() {
            classes_by_hi32.insert(h.to_string(), c);
        }
    }

    for tag in &["09", "0F"] {
        let mut total = 0;
        let mut matched = 0;
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
            // Ref value format: 4000_0000_XXXX_XXXX (likely high 32 bits constant)
            // Take low32 of the ref and check if it matches a class hi32
            let low32 = &rv[8..];
            if classes_by_hi32.contains_key(low32) {
                matched += 1;
                if samples.len() < 8 {
                    let c = classes_by_hi32.get(low32).unwrap();
                    samples.push(format!(
                        "prop {} (type={}) -> ref {} -> class {} (hi32={})",
                        p["id_hex"].as_str().unwrap_or(""),
                        tag,
                        rv,
                        c["class_id_hex"].as_str().unwrap_or(""),
                        c["class_type_hi32"].as_str().unwrap_or("")
                    ));
                }
            }
        }
        println!(
            "\nTag={}: {}/{} ref values match a class via low32==class.hi32 ({:.1}%)",
            tag,
            matched,
            total,
            100.0 * matched as f64 / total as f64
        );
        for s in &samples {
            println!("  {}", s);
        }
    }

    Ok(())
}
