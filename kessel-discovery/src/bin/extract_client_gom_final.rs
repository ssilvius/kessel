//! Final consolidated decoder for client.gom.
//!
//! Walks every record once, builds full cross-reference tables, and emits:
//!   /tmp/client-gom-properties.json           (10006 property records, with type-ref resolution)
//!   /tmp/client-gom-properties-by-system.json (root-class clusters)
//!   /tmp/client-gom-classes.json              (2220 classes with property template GUID list)
//!
//! Property record body format (verified across all 10006 records):
//!   0x00-0x03  u32 size                (33-63)
//!   0x04-0x0B  u64 id (canonical GOM property GUID -- LE order; ALL UNIQUE)
//!   0x0C-0x0F  u32 f0c (registration index)
//!   0x10-0x11  u16 f10 (0x009A or 0x009E)
//!   0x12-0x13  u16 first_str_off = 0x0020
//!   0x14-0x17  u32 (offset markers, 0x001F001E)
//!   0x18-0x19  u16 flags_a
//!   0x1A-0x1B  u16 flags_b
//!   0x1C-0x1D  u16 payload_offset (= 0x0020)
//!   0x1E-0x1F  u16 zero
//!   0x20..end  typed tail = [type_tag u8] [encoded value]
//!
//! Type tag taxonomy (verified):
//!   0x01 bool         (1 byte tail; sz=33)        814 records
//!   0x02 int8         (1 byte tail; sz=33)       1777 records
//!   0x03 int16        (1 byte tail; sz=33)       1192 records
//!   0x04 int32        (1 byte tail; sz=33)        974 records
//!   0x05 enum_ref     (8 bytes; sz=41) -- 100% match enum.low32   510 records
//!   0x06 float32      (1 byte tail; sz=33)        869 records
//!   0x07 string       (1+1+ bytes; sz=34+)        760 records
//!   0x08 array/list   (variable; sz=35-63)       1507 records
//!   0x09 class_ref    (8 bytes; sz=41) -- 100% match class.hi32   169 records
//!   0x0e unknown      (1 byte tail; sz=33)          9 records
//!   0x0f class_arr    (8 bytes; sz=41) -- 86.7% match class.hi32  708 records
//!   0x11 unknown      (1 byte tail; sz=33)        133 records
//!   0x12 unknown      (1 byte tail; sz=33)        342 records
//!   0x14 unknown      (1 byte tail; sz=33)         63 records
//!   0x15 unknown      (1 byte tail; sz=33)        179 records
//!
//! Class record body format:
//!   0x00-0x03  u32 size
//!   0x04-0x0B  u64 class_id (high32 IS the canonical class type tag, e.g. D954FB01 for tal.*)
//!   0x0C-0x0F  u32 f0c (0x40000040)
//!   0x10-0x11  u16 f10 (0x00A2 or 0x00A6)
//!   0x12-0x13  u16 first_str_off = 0x0034
//!   0x14-0x17  u32 (offset markers, 0x00330032)
//!   0x18-0x1F  u64 D000-prefix GUID (native/vtable ref a)
//!   0x20-0x27  u64 D000-prefix GUID (native/vtable ref b)
//!   0x28-0x29  u16 flag1
//!   0x2A-0x2B  u16 prop_count_b
//!   0x2C-0x2D  u16 list_start (= 0x38)
//!   0x2E-0x2F  u16 prop_count (canonical declared count)
//!   0x30-0x37  u64 padding/typeinfo (0x40 high byte)
//!   0x38..end  [u64 template_guid_ref] * N -- template GUIDs (4000XXXX format)
//!
//! Names: NO field/property names are stored in client.gom. The 8-byte property
//! id IS the canonical "field hash" within the GOM type system. To label fields
//! by human-readable name, an EXTERNAL source (e.g. reverse-engineered names
//! from the SWTOR client binary, or community Jedipedia data) is required.

use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;

#[derive(Serialize, Clone)]
struct PropertyRecord {
    index: usize,
    offset_hex: String,
    size: usize,
    /// Canonical 64-bit property GUID (LE order in file). This IS the field hash.
    id_hex: String,
    f0c_hex: String,
    f10_hex: String,
    flags_a_hex: String,
    flags_b_hex: String,
    type_tag: String,
    type_tag_decimal: u8,
    /// Semantic interpretation of the tail's type tag.
    type_kind: String,
    /// Hex dump of the typed tail (everything from 0x20 onward).
    typed_value_hex: String,
    /// If type is 05/09/0F: the 8-byte reference value.
    ref_value_hex: Option<String>,
    /// If type=05: the resolved enum name (members[0] of matching enum).
    resolved_enum_name: Option<String>,
    /// If type=05: full id of the matched enum.
    resolved_enum_id: Option<String>,
    /// If type=05: full member list of the enum.
    resolved_enum_members: Option<Vec<String>>,
    /// If type=09/0F: the matched class id.
    resolved_class_id: Option<String>,
    /// If type=09/0F: which GOM system the class belongs to (well-known root only).
    resolved_class_system: Option<String>,
}

#[derive(Serialize, Clone)]
struct ClassRecord {
    index: usize,
    offset_hex: String,
    size: usize,
    class_id_hex: String,
    class_type_hi32: String,
    /// If hi32 matches a well-known GOM root class, the system name.
    well_known_system: Option<String>,
    parent_a_hex: String,
    parent_b_hex: String,
    prop_count: u16,
    /// Property template GUIDs (4000XXXX format). Length usually = prop_count + 1.
    property_refs: Vec<String>,
}

#[derive(Serialize)]
struct SystemCluster {
    system: String,
    class_id: String,
    class_type_hi32: String,
    parent_a: String,
    parent_b: String,
    prop_count: usize,
    property_refs: Vec<String>,
    related_class_ids: Vec<String>,
}

fn type_kind(tag: u8) -> &'static str {
    match tag {
        0x01 => "bool",
        0x02 => "int8",
        0x03 => "int16",
        0x04 => "int32",
        0x05 => "enum_ref",
        0x06 => "float32",
        0x07 => "string",
        0x08 => "array",
        0x09 => "class_ref_strong",
        0x0e => "unknown_0E",
        0x0f => "class_ref_weak",
        0x11 => "unknown_11",
        0x12 => "unknown_12",
        0x14 => "unknown_14",
        0x15 => "unknown_15",
        _ => "unknown",
    }
}

fn well_known_system(hi32: &str) -> Option<&'static str> {
    match hi32 {
        "D954FB01" => Some("tal (Talent)"),
        "0283F4D2" => Some("abl (Ability)"),
        "011ACD0E" => Some("itm (Item)"),
        "0078E1BD" => Some("npc (Npc)"),
        "F9E467C7" => Some("mpn (MissionPoint)"),
        "2ADEC3D2" => Some("qst (Quest)"),
        "257639EC" => Some("cdx (Codex)"),
        "3AC53EA0" => Some("ach (Achievement)"),
        "DFA8408A" => Some("schem (Schematic)"),
        _ => None,
    }
}

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let enums: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-dict.json")?)?;

    // Build enum lookup: enum.hash.low32 -> (name, full_id, members)
    let mut enum_by_low32: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    for e in &enums {
        if let Some(h) = e["hash"].as_str() {
            if h.len() == 16 {
                let low32 = h[8..].to_string();
                let name = e["name"].as_str().unwrap_or("").to_string();
                let members: Vec<String> = e["members"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                enum_by_low32.insert(low32, (name, h.to_string(), members));
            }
        }
    }

    let mut pos: usize = 8;
    let mut idx = 0;
    let mut classes_raw: Vec<ClassRecord> = Vec::new();
    let mut props_raw: Vec<PropertyRecord> = Vec::new();

    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        if size == 0 {
            pos += 1;
            continue;
        }
        if size < 16 || pos + size > data.len() {
            break;
        }
        let body = &data[pos..pos + size];

        if body.len() >= 0x21 {
            let fso = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            let id = u64::from_le_bytes(body[0x04..0x0C].try_into()?);
            let f0c = u32::from_le_bytes(body[0x0C..0x10].try_into()?);
            let f10 = u16::from_le_bytes(body[0x10..0x12].try_into()?);

            if fso == 0x20 {
                let flags_a = u16::from_le_bytes(body[0x18..0x1A].try_into()?);
                let flags_b = u16::from_le_bytes(body[0x1A..0x1C].try_into()?);
                let type_tag = body[0x20];
                let tail = &body[0x20..];
                let typed_hex: String = tail.iter().map(|b| format!("{:02X}", b)).collect();
                let ref_value = match type_tag {
                    0x05 | 0x09 | 0x0F if tail.len() >= 9 => {
                        let v = u64::from_le_bytes(tail[1..9].try_into()?);
                        Some(format!("{:016X}", v))
                    }
                    _ => None,
                };

                let mut prop = PropertyRecord {
                    index: idx,
                    offset_hex: format!("{:#08x}", pos),
                    size,
                    id_hex: format!("{:016X}", id),
                    f0c_hex: format!("{:08X}", f0c),
                    f10_hex: format!("{:04X}", f10),
                    flags_a_hex: format!("{:04X}", flags_a),
                    flags_b_hex: format!("{:04X}", flags_b),
                    type_tag: format!("{:02X}", type_tag),
                    type_tag_decimal: type_tag,
                    type_kind: type_kind(type_tag).to_string(),
                    typed_value_hex: typed_hex,
                    ref_value_hex: ref_value.clone(),
                    resolved_enum_name: None,
                    resolved_enum_id: None,
                    resolved_enum_members: None,
                    resolved_class_id: None,
                    resolved_class_system: None,
                };

                // Enum resolution for type=05
                if let Some(rv) = &ref_value {
                    if rv.len() == 16 && type_tag == 0x05 {
                        let low32 = &rv[8..];
                        if let Some((name, full, members)) = enum_by_low32.get(low32) {
                            prop.resolved_enum_name = Some(name.clone());
                            prop.resolved_enum_id = Some(full.clone());
                            prop.resolved_enum_members = Some(members.clone());
                        }
                    }
                }
                props_raw.push(prop);
            } else if fso == 0x34 && body.len() >= 0x38 {
                let parent_a = u64::from_le_bytes(body[0x18..0x20].try_into()?);
                let parent_b = u64::from_le_bytes(body[0x20..0x28].try_into()?);
                let prop_count = u16::from_le_bytes(body[0x2E..0x30].try_into()?);
                let mut refs = Vec::new();
                let mut i = 0x38;
                while i + 8 <= size {
                    let v = u64::from_le_bytes(body[i..i + 8].try_into()?);
                    refs.push(format!("{:016X}", v));
                    i += 8;
                }
                let class_id_hex = format!("{:016X}", id);
                let hi32 = format!("{:08X}", (id >> 32) as u32);
                let ws = well_known_system(&hi32).map(String::from);
                classes_raw.push(ClassRecord {
                    index: idx,
                    offset_hex: format!("{:#08x}", pos),
                    size,
                    class_id_hex,
                    class_type_hi32: hi32,
                    well_known_system: ws,
                    parent_a_hex: format!("{:016X}", parent_a),
                    parent_b_hex: format!("{:016X}", parent_b),
                    prop_count,
                    property_refs: refs,
                });
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }

    // Build class-by-hi32 index for 09/0F resolution
    let class_by_hi32: HashMap<String, &ClassRecord> = classes_raw
        .iter()
        .map(|c| (c.class_type_hi32.clone(), c))
        .collect();
    for p in props_raw.iter_mut() {
        if let Some(rv) = &p.ref_value_hex {
            if rv.len() == 16 && (p.type_tag_decimal == 0x09 || p.type_tag_decimal == 0x0F) {
                let low32 = &rv[8..];
                if let Some(c) = class_by_hi32.get(low32) {
                    p.resolved_class_id = Some(c.class_id_hex.clone());
                    p.resolved_class_system = c.well_known_system.clone();
                }
            }
        }
    }

    println!(
        "Decoded {} property records, {} class records",
        props_raw.len(),
        classes_raw.len()
    );

    let pj = serde_json::to_string_pretty(&props_raw)?;
    fs::write("/tmp/client-gom-properties.json", &pj)?;
    println!("Wrote /tmp/client-gom-properties.json ({} bytes)", pj.len());

    let cj = serde_json::to_string_pretty(&classes_raw)?;
    fs::write("/tmp/client-gom-classes.json", &cj)?;
    println!("Wrote /tmp/client-gom-classes.json ({} bytes)", cj.len());

    // System clustering
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

    let mut systems_out: Vec<SystemCluster> = Vec::new();
    for (hi32, sys_name) in &systems {
        let Some(root) = classes_raw.iter().find(|c| c.class_type_hi32 == *hi32) else {
            continue;
        };
        let ref_set: std::collections::BTreeSet<&String> = root.property_refs.iter().collect();
        let related: Vec<String> = classes_raw
            .iter()
            .filter(|c| {
                c.class_id_hex != root.class_id_hex
                    && c.property_refs.iter().any(|r| ref_set.contains(r))
            })
            .map(|c| c.class_id_hex.clone())
            .collect();
        systems_out.push(SystemCluster {
            system: sys_name.to_string(),
            class_id: root.class_id_hex.clone(),
            class_type_hi32: root.class_type_hi32.clone(),
            parent_a: root.parent_a_hex.clone(),
            parent_b: root.parent_b_hex.clone(),
            prop_count: root.prop_count as usize,
            property_refs: root.property_refs.clone(),
            related_class_ids: related,
        });
    }

    let bj = serde_json::to_string_pretty(&systems_out)?;
    fs::write("/tmp/client-gom-properties-by-system.json", &bj)?;
    println!(
        "Wrote /tmp/client-gom-properties-by-system.json ({} bytes)",
        bj.len()
    );

    // Stats output
    let mut type_kind_freq: BTreeMap<&str, usize> = BTreeMap::new();
    let mut enum_resolved = 0;
    let mut class_resolved = 0;
    for p in &props_raw {
        *type_kind_freq
            .entry(type_kind(p.type_tag_decimal))
            .or_insert(0) += 1;
        if p.resolved_enum_name.is_some() {
            enum_resolved += 1;
        }
        if p.resolved_class_id.is_some() {
            class_resolved += 1;
        }
    }
    println!("\n=== Type-kind distribution ===");
    for (k, n) in &type_kind_freq {
        println!("  {:20}  {}", k, n);
    }
    println!("\n=== Resolution stats ===");
    println!("  Type=05 (enum_ref) -> resolved enum: {}", enum_resolved);
    println!(
        "  Type=09|0F (class_ref) -> resolved class: {}",
        class_resolved
    );

    Ok(())
}
