//! Cross-reference: properties with type=05 ref their value's low32 against enum ids' low32.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let enums: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-dict.json")?)?;

    // Index enums by low32 of their hash
    let mut enum_by_low32: HashMap<String, &Value> = HashMap::new();
    for e in &enums {
        if let Some(h) = e["hash"].as_str() {
            if h.len() == 16 {
                let low32 = h[8..].to_string();
                enum_by_low32.insert(low32, e);
            }
        }
    }

    let mut t05_total = 0;
    let mut t05_match_enum = 0;
    let mut t09_total = 0;
    let mut t09_match_enum = 0;
    let mut t0f_total = 0;
    let mut t0f_match_enum = 0;
    let mut sample_05_matches: Vec<String> = Vec::new();
    let mut sample_09_matches: Vec<String> = Vec::new();
    let mut sample_0f_matches: Vec<String> = Vec::new();

    for p in &props {
        let Some(tag) = p["type_tag"].as_str() else {
            continue;
        };
        let Some(rv) = p["ref_value_hex"].as_str() else {
            continue;
        };
        if rv.len() != 16 {
            continue;
        }
        let low32 = &rv[8..];

        let matches_enum = enum_by_low32.contains_key(low32);

        match tag {
            "05" => {
                t05_total += 1;
                if matches_enum {
                    t05_match_enum += 1;
                    if sample_05_matches.len() < 5 {
                        let e = enum_by_low32.get(low32).unwrap();
                        sample_05_matches.push(format!(
                            "prop {} -> enum '{}' (id={})",
                            p["id_hex"].as_str().unwrap_or(""),
                            e["name"].as_str().unwrap_or(""),
                            e["hash"].as_str().unwrap_or("")
                        ));
                    }
                }
            }
            "09" => {
                t09_total += 1;
                if matches_enum {
                    t09_match_enum += 1;
                    if sample_09_matches.len() < 5 {
                        let e = enum_by_low32.get(low32).unwrap();
                        sample_09_matches.push(format!(
                            "prop {} -> enum '{}' (id={})",
                            p["id_hex"].as_str().unwrap_or(""),
                            e["name"].as_str().unwrap_or(""),
                            e["hash"].as_str().unwrap_or("")
                        ));
                    }
                }
            }
            "0F" => {
                t0f_total += 1;
                if matches_enum {
                    t0f_match_enum += 1;
                    if sample_0f_matches.len() < 5 {
                        let e = enum_by_low32.get(low32).unwrap();
                        sample_0f_matches.push(format!(
                            "prop {} -> enum '{}' (id={})",
                            p["id_hex"].as_str().unwrap_or(""),
                            e["name"].as_str().unwrap_or(""),
                            e["hash"].as_str().unwrap_or("")
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    println!(
        "Type=05 props: {} total, {} match an enum by low32",
        t05_total, t05_match_enum
    );
    println!(
        "Type=09 props: {} total, {} match an enum by low32",
        t09_total, t09_match_enum
    );
    println!(
        "Type=0F props: {} total, {} match an enum by low32",
        t0f_total, t0f_match_enum
    );
    println!("\n05 samples:");
    for s in &sample_05_matches {
        println!("  {}", s);
    }
    println!("\n09 samples:");
    for s in &sample_09_matches {
        println!("  {}", s);
    }
    println!("\n0F samples:");
    for s in &sample_0f_matches {
        println!("  {}", s);
    }

    Ok(())
}
