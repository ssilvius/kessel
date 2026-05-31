//! For each well-known GOM system, fully decode its property template list:
//! template_ref -> property record -> type info & enum/class resolution.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;

#[derive(Serialize)]
struct PropDetail {
    template_ref: String,
    inner_hi32: String,
    property_id: Option<String>,
    type_kind: Option<String>,
    type_tag: Option<String>,
    typed_value_hex: Option<String>,
    ref_value_hex: Option<String>,
    resolved_enum_name: Option<String>,
    resolved_enum_id: Option<String>,
    resolved_enum_first_members: Option<Vec<String>>,
    resolved_class_id: Option<String>,
    resolved_class_system: Option<String>,
}

#[derive(Serialize)]
struct SystemDetailed {
    system: String,
    class_id: String,
    prop_count: u16,
    properties: Vec<PropDetail>,
}

fn main() -> anyhow::Result<()> {
    let props: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-properties.json")?)?;
    let classes: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-classes.json")?)?;

    let mut by_hi32: HashMap<String, &Value> = HashMap::new();
    for p in &props {
        if let Some(id) = p["id_hex"].as_str() {
            if id.len() >= 8 {
                by_hi32.insert(id[..8].to_string(), p);
            }
        }
    }

    let systems = [
        "D954FB01", "0283F4D2", "011ACD0E", "0078E1BD", "F9E467C7", "2ADEC3D2", "257639EC",
        "3AC53EA0", "DFA8408A",
    ];
    let mut out: Vec<SystemDetailed> = Vec::new();

    for sys in &systems {
        let Some(c) = classes
            .iter()
            .find(|c| c["class_type_hi32"].as_str() == Some(sys))
        else {
            continue;
        };
        let sys_name = c["well_known_system"].as_str().unwrap_or("").to_string();
        let cid = c["class_id_hex"].as_str().unwrap_or("").to_string();
        let prop_count = c["prop_count"].as_u64().unwrap_or(0) as u16;
        let refs: Vec<String> = c["property_refs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        let mut details = Vec::new();
        for r in &refs {
            let inner = r[8..].to_string();
            let p = by_hi32.get(&inner);
            let pd = PropDetail {
                template_ref: r.clone(),
                inner_hi32: inner.clone(),
                property_id: p.and_then(|p| p["id_hex"].as_str().map(String::from)),
                type_kind: p.and_then(|p| p["type_kind"].as_str().map(String::from)),
                type_tag: p.and_then(|p| p["type_tag"].as_str().map(String::from)),
                typed_value_hex: p.and_then(|p| p["typed_value_hex"].as_str().map(String::from)),
                ref_value_hex: p.and_then(|p| p["ref_value_hex"].as_str().map(String::from)),
                resolved_enum_name: p
                    .and_then(|p| p["resolved_enum_name"].as_str().map(String::from)),
                resolved_enum_id: p.and_then(|p| p["resolved_enum_id"].as_str().map(String::from)),
                resolved_enum_first_members: p.and_then(|p| {
                    p["resolved_enum_members"].as_array().map(|a| {
                        a.iter()
                            .take(6)
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                }),
                resolved_class_id: p
                    .and_then(|p| p["resolved_class_id"].as_str().map(String::from)),
                resolved_class_system: p
                    .and_then(|p| p["resolved_class_system"].as_str().map(String::from)),
            };
            details.push(pd);
        }

        out.push(SystemDetailed {
            system: sys_name,
            class_id: cid,
            prop_count,
            properties: details,
        });
    }

    let j = serde_json::to_string_pretty(&out)?;
    fs::write("/tmp/client-gom-system-schemas.json", &j)?;
    println!(
        "Wrote /tmp/client-gom-system-schemas.json ({} bytes)",
        j.len()
    );

    // Print quick summary
    for s in &out {
        let typed_count = s
            .properties
            .iter()
            .filter(|p| p.type_kind.is_some())
            .count();
        let enum_count = s
            .properties
            .iter()
            .filter(|p| p.resolved_enum_name.is_some())
            .count();
        let class_count = s
            .properties
            .iter()
            .filter(|p| p.resolved_class_id.is_some())
            .count();
        println!(
            "{}: {} declared props, {} resolved via prop records ({} enum-typed, {} class-typed)",
            s.system, s.prop_count, typed_count, enum_count, class_count
        );
    }

    Ok(())
}
