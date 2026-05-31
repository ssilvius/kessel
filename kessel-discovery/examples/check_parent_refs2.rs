//! Check if parent_a/parent_b match property record IDs.

use serde_json::Value;
use std::collections::HashSet;
use std::fs;

fn main() -> anyhow::Result<()> {
    let classes_raw = fs::read_to_string("/tmp/client-gom-classes.json")?;
    let classes: Vec<Value> = serde_json::from_str(&classes_raw)?;
    let props_raw = fs::read_to_string("/tmp/client-gom-properties.json")?;
    let props: Vec<Value> = serde_json::from_str(&props_raw)?;

    let prop_ids: HashSet<String> = props
        .iter()
        .filter_map(|p| p["id_hex"].as_str().map(String::from))
        .collect();

    // Also from enum dict
    let enum_raw = fs::read_to_string("/tmp/client-gom-dict.json").unwrap_or_default();
    let enums: Vec<Value> = serde_json::from_str(&enum_raw).unwrap_or_default();
    let enum_ids: HashSet<String> = enums
        .iter()
        .filter_map(|e| e["hash"].as_str().map(String::from))
        .collect();

    let mut pa_in_props = 0;
    let mut pa_in_enums = 0;
    let mut pb_in_props = 0;
    let mut pb_in_enums = 0;

    for c in &classes {
        let pa = c["parent_a_hex"].as_str().unwrap_or("").to_string();
        let pb = c["parent_b_hex"].as_str().unwrap_or("").to_string();
        if prop_ids.contains(&pa) {
            pa_in_props += 1;
        }
        if enum_ids.contains(&pa) {
            pa_in_enums += 1;
        }
        if prop_ids.contains(&pb) {
            pb_in_props += 1;
        }
        if enum_ids.contains(&pb) {
            pb_in_enums += 1;
        }
    }

    println!("Total classes: {}", classes.len());
    println!("parent_a in prop ids: {}", pa_in_props);
    println!("parent_a in enum ids: {}", pa_in_enums);
    println!("parent_b in prop ids: {}", pb_in_props);
    println!("parent_b in enum ids: {}", pb_in_enums);

    // What do parent prefixes look like?
    use std::collections::BTreeMap;
    let mut pa_prefix: BTreeMap<String, usize> = BTreeMap::new();
    let mut pb_prefix: BTreeMap<String, usize> = BTreeMap::new();
    for c in &classes {
        let pa = c["parent_a_hex"].as_str().unwrap_or("").to_string();
        let pb = c["parent_b_hex"].as_str().unwrap_or("").to_string();
        if pa.len() >= 4 {
            *pa_prefix.entry(pa[..4].to_string()).or_insert(0) += 1;
        }
        if pb.len() >= 4 {
            *pb_prefix.entry(pb[..4].to_string()).or_insert(0) += 1;
        }
    }
    println!("\nparent_a hex-prefix distribution (first 4 chars):");
    for (p, n) in &pa_prefix {
        println!("  {} -> {}", p, n);
    }
    println!("\nparent_b hex-prefix distribution:");
    for (p, n) in &pb_prefix {
        println!("  {} -> {}", p, n);
    }

    Ok(())
}
