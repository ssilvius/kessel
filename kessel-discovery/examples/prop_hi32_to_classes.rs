//! Check if property record IDs' hi32 matches class IDs' hi32 (or class IDs themselves).

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let classes: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-classes.json")?)?;

    let class_hi32_set: HashSet<String> = classes
        .iter()
        .filter_map(|c| c["class_type_hi32"].as_str().map(String::from))
        .collect();
    let class_hi32_to: HashMap<String, &Value> = classes
        .iter()
        .filter_map(|c| c["class_type_hi32"].as_str().map(|s| (s.to_string(), c)))
        .collect();

    let mut matched = 0;
    let mut total = 0;
    let mut samples: Vec<String> = Vec::new();
    for p in &props {
        let Some(id) = p["id_hex"].as_str() else {
            continue;
        };
        if id.len() != 16 {
            continue;
        }
        total += 1;
        let hi32 = &id[..8];
        if class_hi32_set.contains(hi32) {
            matched += 1;
            if samples.len() < 10 {
                let c = class_hi32_to.get(hi32).unwrap();
                samples.push(format!(
                    "prop {} (hi32={}) -> class {} (full={})",
                    id,
                    hi32,
                    c["class_id_hex"].as_str().unwrap_or(""),
                    c["class_id_hex"].as_str().unwrap_or("")
                ));
            }
        }
    }

    println!(
        "Property ID hi32 matches a class hi32: {}/{} ({:.1}%)",
        matched,
        total,
        100.0 * matched as f64 / total as f64
    );
    for s in &samples {
        println!("  {}", s);
    }

    // Reverse: how many distinct hi32 values among property IDs?
    use std::collections::BTreeMap;
    let mut hi32_freq: BTreeMap<String, usize> = BTreeMap::new();
    for p in &props {
        if let Some(id) = p["id_hex"].as_str() {
            hi32_freq
                .entry(id[..8].to_string())
                .and_modify(|v| *v += 1)
                .or_insert(1);
        }
    }
    println!(
        "\n{} distinct hi32 values among 10006 properties",
        hi32_freq.len()
    );

    let mut v: Vec<_> = hi32_freq.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nTop hi32 values:");
    for (h, n) in v.iter().take(15) {
        let in_classes = class_hi32_set.contains(h);
        println!("  hi32={} count={} in_classes={}", h, n, in_classes);
    }

    Ok(())
}
