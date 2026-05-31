//! Build a per-system property cluster.
//!
//! Strategy:
//! 1. Read classes.json. Find the well-known root classes by class_type_hi32
//!    (D954FB01=tal, 0283F4D2=abl, 011ACD0E=itm, etc.).
//! 2. For each root class, gather its property_refs.
//! 3. Cross-reference property_refs against ALL class records to find usage maps.
//! 4. Emit /tmp/client-gom-properties-by-system.json.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[derive(Serialize)]
struct SystemCluster {
    system: String,
    class_id: String,
    class_type_hi32: String,
    parent_a: String,
    parent_b: String,
    prop_count: usize,
    property_refs: Vec<String>,
    /// other classes that share at least one property with this one
    related_class_ids: Vec<String>,
}

#[derive(Serialize)]
struct PropertyUsage {
    property_ref: String,
    used_by_class_ids: Vec<String>,
    /// usage count
    usage_count: usize,
}

fn main() -> anyhow::Result<()> {
    let classes_raw = fs::read_to_string("/tmp/client-gom-classes.json")?;
    let classes: Vec<Value> = serde_json::from_str(&classes_raw)?;

    // Build: for every class, its (class_id, parent_a, parent_b, prop_refs)
    let class_list: Vec<(String, String, String, String, u16, Vec<String>)> = classes
        .iter()
        .map(|c| {
            let class_id = c["class_id_hex"].as_str().unwrap_or("").to_string();
            let class_hi = c["class_type_hi32"].as_str().unwrap_or("").to_string();
            let pa = c["parent_a_hex"].as_str().unwrap_or("").to_string();
            let pb = c["parent_b_hex"].as_str().unwrap_or("").to_string();
            let cnt = c["prop_count"].as_u64().unwrap_or(0) as u16;
            let refs: Vec<String> = c["property_refs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (class_id, class_hi, pa, pb, cnt, refs)
        })
        .collect();

    // Well-known root classes (class_type_hi32 -> system name)
    let systems: Vec<(&str, &str)> = vec![
        ("D954FB01", "tal (Talent)"),
        ("0283F4D2", "abl (Ability)"),
        ("011ACD0E", "itm (Item)"),
        ("0078E1BD", "npc (Npc)"),
        ("F9E467C7", "mpn (MissionPoint)"),
        ("2ADEC3D2", "qst (Quest)"),
        ("257639EC", "cdx (Codex)"),
        ("3AC53EA0", "ach (Achievement)"),
        ("DFA8408A", "schem (Schematic)"),
    ];

    // For each well-known root class, find its record
    let mut systems_out: Vec<SystemCluster> = Vec::new();
    for (hi32, sys_name) in &systems {
        let Some(root) = class_list.iter().find(|(_, h, _, _, _, _)| h == hi32) else {
            eprintln!("warning: no class found for {}", hi32);
            continue;
        };
        let (cid, _, pa, pb, cnt, refs) = root;

        // Find related classes that share any property
        let ref_set: BTreeSet<&String> = refs.iter().collect();
        let related: Vec<String> = class_list
            .iter()
            .filter(|(other_id, _, _, _, _, other_refs)| {
                other_id != cid && other_refs.iter().any(|r| ref_set.contains(r))
            })
            .map(|(other_id, _, _, _, _, _)| other_id.clone())
            .collect();

        systems_out.push(SystemCluster {
            system: sys_name.to_string(),
            class_id: cid.clone(),
            class_type_hi32: hi32.to_string(),
            parent_a: pa.clone(),
            parent_b: pb.clone(),
            prop_count: *cnt as usize,
            property_refs: refs.clone(),
            related_class_ids: related,
        });
    }

    let by_system = serde_json::to_string_pretty(&systems_out)?;
    fs::write("/tmp/client-gom-properties-by-system.json", &by_system)?;
    println!(
        "Wrote /tmp/client-gom-properties-by-system.json ({} bytes)",
        by_system.len()
    );

    // Build per-property usage map (which classes USE each property)
    let mut usage_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cid, _, _, _, _, refs) in &class_list {
        for r in refs {
            usage_map.entry(r.clone()).or_default().push(cid.clone());
        }
    }
    let mut usage_list: Vec<PropertyUsage> = usage_map
        .into_iter()
        .map(|(prop, classes)| PropertyUsage {
            property_ref: prop,
            usage_count: classes.len(),
            used_by_class_ids: classes,
        })
        .collect();
    usage_list.sort_by(|a, b| b.usage_count.cmp(&a.usage_count));

    let usage_json = serde_json::to_string_pretty(&usage_list)?;
    fs::write("/tmp/client-gom-property-usage.json", &usage_json)?;
    println!(
        "Wrote /tmp/client-gom-property-usage.json ({} bytes, {} unique props)",
        usage_json.len(),
        usage_list.len()
    );

    // Print summary
    println!("\n=== Per-system summary ===");
    for sys in &systems_out {
        println!(
            "  {} (class {} hi32={}) prop_count={} related_classes={}",
            sys.system,
            sys.class_id,
            sys.class_type_hi32,
            sys.prop_count,
            sys.related_class_ids.len()
        );
    }

    println!("\n=== Top 10 most-referenced properties (across all 2220 classes) ===");
    for u in usage_list.iter().take(10) {
        println!("  {} used by {} classes", u.property_ref, u.usage_count);
    }

    Ok(())
}
