//! Check if class parent_a/parent_b refs point to other class records.

use serde_json::Value;
use std::collections::HashSet;
use std::fs;

fn main() -> anyhow::Result<()> {
    let classes_raw = fs::read_to_string("/tmp/client-gom-classes.json")?;
    let classes: Vec<Value> = serde_json::from_str(&classes_raw)?;

    let class_ids: HashSet<String> = classes
        .iter()
        .filter_map(|c| c["class_id_hex"].as_str().map(String::from))
        .collect();

    let mut parent_a_in_set = 0;
    let mut parent_b_in_set = 0;
    let mut both_zero = 0;
    let mut a_zero_only = 0;
    let mut neither_in_set = 0;

    for c in &classes {
        let pa = c["parent_a_hex"].as_str().unwrap_or("0").to_string();
        let pb = c["parent_b_hex"].as_str().unwrap_or("0").to_string();
        let a_z = pa == "0000000000000000";
        let b_z = pb == "0000000000000000";
        if a_z && b_z {
            both_zero += 1;
            continue;
        }
        if a_z {
            a_zero_only += 1;
        }
        let a_in = !a_z && class_ids.contains(&pa);
        let b_in = !b_z && class_ids.contains(&pb);
        if a_in {
            parent_a_in_set += 1;
        }
        if b_in {
            parent_b_in_set += 1;
        }
        if !a_z && !a_in && !b_z && !b_in {
            neither_in_set += 1;
        }
    }

    println!("Total classes: {}", classes.len());
    println!("Parent A in class set: {}", parent_a_in_set);
    println!("Parent B in class set: {}", parent_b_in_set);
    println!("Both parents zero: {}", both_zero);
    println!("A=0, B set: {}", a_zero_only);
    println!("Neither parent in class set: {}", neither_in_set);

    // Print first 5 examples of each
    println!("\n--- First 5 classes with both parents in set ---");
    let mut shown = 0;
    for c in classes.iter() {
        let pa = c["parent_a_hex"].as_str().unwrap_or("").to_string();
        let pb = c["parent_b_hex"].as_str().unwrap_or("").to_string();
        if class_ids.contains(&pa) && class_ids.contains(&pb) {
            println!(
                "  {} -> parent_a={} parent_b={}",
                c["class_id_hex"].as_str().unwrap_or(""),
                pa,
                pb
            );
            shown += 1;
            if shown >= 5 {
                break;
            }
        }
    }

    Ok(())
}
