//! Get distribution of distinct ref values for type=09 and type=0F properties.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;

    for tag in &["09", "0F"] {
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();
        for p in &props {
            if p["type_tag"].as_str() != Some(tag) {
                continue;
            }
            if let Some(rv) = p["ref_value_hex"].as_str() {
                *freq.entry(rv.to_string()).or_insert(0) += 1;
            }
        }
        let mut v: Vec<_> = freq.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "\n=== type={} ref_value distribution (top 30 of {} unique) ===",
            tag,
            v.len()
        );
        for (rv, n) in v.iter().take(30) {
            println!("  {} -> {}", rv, n);
        }
    }

    Ok(())
}
