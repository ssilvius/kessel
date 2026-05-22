//! For each property_ref in classes, check whether it matches a property record id.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> anyhow::Result<()> {
    let classes: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-classes.json")?)?;
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;

    let prop_ids: HashSet<String> = props
        .iter()
        .filter_map(|p| p["id_hex"].as_str().map(String::from))
        .collect();
    let prop_id_to_record: HashMap<String, &Value> = props
        .iter()
        .filter_map(|p| p["id_hex"].as_str().map(|s| (s.to_string(), p)))
        .collect();

    let mut total_refs = 0;
    let mut matched_refs = 0;
    let mut sample_unmatched: Vec<String> = Vec::new();
    let mut sample_matched: Vec<String> = Vec::new();

    for c in &classes {
        let refs = c["property_refs"].as_array().unwrap();
        for r in refs {
            let s = r.as_str().unwrap().to_string();
            total_refs += 1;
            if prop_ids.contains(&s) {
                matched_refs += 1;
                if sample_matched.len() < 5 {
                    sample_matched.push(s.clone());
                }
            } else if sample_unmatched.len() < 10 {
                sample_unmatched.push(s);
            }
        }
    }

    println!("Total class property_refs: {}", total_refs);
    println!(
        "Matched (ref points to a property record): {} ({:.1}%)",
        matched_refs,
        100.0 * matched_refs as f64 / total_refs as f64
    );
    println!("\nSample MATCHED refs (these point to known property records):");
    for s in &sample_matched {
        let p = prop_id_to_record.get(s).unwrap();
        println!(
            "  {} -> type_tag={} size={}",
            s,
            p["type_tag"].as_str().unwrap_or(""),
            p["size"].as_u64().unwrap_or(0)
        );
    }
    println!("\nSample UNMATCHED refs (not in prop records):");
    for s in &sample_unmatched {
        println!("  {}", s);
    }

    Ok(())
}
