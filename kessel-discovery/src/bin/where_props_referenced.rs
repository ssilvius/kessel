//! Find where property record IDs are referenced.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let prop_ids: HashSet<String> = props
        .iter()
        .filter_map(|p| p["id_hex"].as_str().map(String::from))
        .collect();

    // Search whether any property's ref_value points to another property
    let mut p2p = 0;
    let mut p2other = 0;
    let mut p2other_examples: Vec<String> = Vec::new();
    for p in &props {
        if let Some(r) = p["ref_value_hex"].as_str() {
            if prop_ids.contains(r) {
                p2p += 1;
            } else {
                p2other += 1;
                if p2other_examples.len() < 10 {
                    p2other_examples.push(format!(
                        "{} (type={}) -> {}",
                        p["id_hex"].as_str().unwrap_or(""),
                        p["type_tag"].as_str().unwrap_or(""),
                        r
                    ));
                }
            }
        }
    }

    println!("Property records with ref_value:");
    println!("  -> another property record: {}", p2p);
    println!("  -> elsewhere: {}", p2other);
    println!("\nExamples of -> elsewhere refs:");
    for e in &p2other_examples {
        println!("  {}", e);
    }

    // Check prefix distribution of ref_values
    use std::collections::BTreeMap;
    let mut ref_prefix: BTreeMap<String, usize> = BTreeMap::new();
    for p in &props {
        if let Some(r) = p["ref_value_hex"].as_str() {
            if r.len() >= 4 {
                *ref_prefix.entry(r[..4].to_string()).or_insert(0) += 1;
            }
        }
    }
    println!("\nProperty ref_value prefix distribution:");
    let mut v: Vec<_> = ref_prefix.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (p, n) in &v {
        println!("  {} -> {}", p, n);
    }

    Ok(())
}
