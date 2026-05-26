//! GOM schema dictionaries baked into the kessel binary as static resources.
//!
//! Source: `/resources/systemgenerated/client.gom` from a SWTOR archive
//! (decoded by sub-agent A and D, legion reflections `019e4d5a` and
//! `019e4d75`). Lazily parsed on first access via `OnceLock`.
//!
//! Three artifacts:
//! - **enums** (748 entries): each is a list of NULL-separated member names
//!   that the game references by ordinal index. Includes `STAT` (517 stats),
//!   `effAction` (205 actions, with 26 `effAction_SPVP*` for GSF), `effParam`,
//!   `effResult`, `scFFComponent`, `scPowerChannel`, `cbtDamage`, etc.
//! - **classes** (2,220 entries): each declares an ordered list of property
//!   templates. 9 are "root systems" (Talent, Ability, Item, Npc,
//!   MissionPoint, Quest, Codex, Achievement, Schematic).
//! - **properties** (10,006 entries): typed declarations with a type tag (bool,
//!   int8/16/32, enum_ref, float32, string, array, class_ref) and optional
//!   resolved enum/class name.
//!
//! Resolution rules:
//! - Class -> property: `class.property_refs[i].low32 == property.id.high32`
//! - Class -> CF40 marker in payload: `class_type_hi32` matches the 4-byte
//!   high32 of the payload's `CF 40 00 00 ...` template GUID
//! - Property -> CF40 marker in payload: `property.id.high32` matches the
//!   4-byte hash following `CF 40 00 00`
//!
//! Regenerate the embedded JSONs by running `kessel-discovery` binaries
//! `extract_client_gom_enums` and `extract_client_gom_final` against a fresh
//! archive, then minifying the output (see `kessel/resources/README` for the
//! regeneration script).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const EMBEDDED_ENUMS: &str = include_str!("../resources/gom_enums.json");
const EMBEDDED_CLASSES: &str = include_str!("../resources/gom_classes.json");
const EMBEDDED_PROPERTIES: &str = include_str!("../resources/gom_properties.json");

/// One GOM enum from client.gom.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GomEnum {
    /// Hex u64 id of the enum record.
    pub hash: String,
    /// Inferred enum name (derived from the first member's common prefix).
    pub name: String,
    /// Ordered member names. `members[i]` is the value the engine references
    /// when payload bytes encode index `i`.
    pub members: Vec<String>,
}

/// One GOM class declaration.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GomClass {
    /// Hex u64 class id.
    pub class_id: String,
    /// Hex u32 high32 of the class id -- matches the CF40 marker high32 in
    /// payloads.
    pub class_type_hi32: String,
    /// Well-known root system name, if any (`Quest`, `Ability`, `Item`, etc.).
    pub name: Option<String>,
    /// Ordered list of property template GUIDs (hex u64 each). Each entry's
    /// low32 matches a property's `id` high32.
    pub property_refs: Vec<String>,
}

/// One GOM property declaration.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GomProperty {
    /// Hex u64 property id.
    pub id: String,
    /// Type-tag byte (0x01..0x15 observed in client.gom).
    pub type_tag: u8,
    /// Human-readable kind: `"bool"`, `"int8"`, `"int16"`, `"int32"`,
    /// `"enum_ref"`, `"float32"`, `"string"`, `"array"`, `"class_ref_strong"`,
    /// `"class_ref_weak"`, or `"unknown"`.
    pub kind: String,
    /// Resolved enum/class references when applicable.
    pub refs: Option<Vec<PropertyRef>>,
}

/// Resolved reference target from a typed property.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PropertyRef {
    /// `"enum"` or `"class"`.
    pub kind: String,
    pub name: String,
}

/// Lazy-loaded schema dictionaries. Use the module-level helper functions
/// (`property_for_cf40`, `class_for_type_hi32`, `enum_for_hash`) rather than
/// holding this struct directly.
pub struct GomSchemaDict {
    /// Enums keyed by their hex hash string.
    pub enums_by_hash: HashMap<String, GomEnum>,
    /// Classes keyed by their hex `class_type_hi32` string.
    pub classes_by_type_hi32: HashMap<String, GomClass>,
    /// Properties keyed by the hex `id` high32 (matches CF40 markers in
    /// payloads). Stored as a fast u32-keyed map for hot-path lookup.
    pub properties_by_id_hi32: HashMap<u32, GomProperty>,
}

static SCHEMA: OnceLock<GomSchemaDict> = OnceLock::new();

/// Singleton accessor. Loads the embedded JSONs on first call.
#[allow(dead_code)]
pub fn schema() -> &'static GomSchemaDict {
    SCHEMA.get_or_init(load)
}

#[allow(dead_code)]
fn load() -> GomSchemaDict {
    let enums: Vec<GomEnum> =
        serde_json::from_str(EMBEDDED_ENUMS).expect("embedded gom_enums.json malformed");
    let classes: Vec<GomClass> =
        serde_json::from_str(EMBEDDED_CLASSES).expect("embedded gom_classes.json malformed");
    let properties: Vec<GomProperty> =
        serde_json::from_str(EMBEDDED_PROPERTIES).expect("embedded gom_properties.json malformed");

    let mut enums_by_hash = HashMap::with_capacity(enums.len());
    for e in enums {
        enums_by_hash.insert(e.hash.clone(), e);
    }

    let mut classes_by_type_hi32 = HashMap::with_capacity(classes.len());
    for c in classes {
        classes_by_type_hi32.insert(c.class_type_hi32.clone(), c);
    }

    let mut properties_by_id_hi32 = HashMap::with_capacity(properties.len());
    for p in properties {
        // id is hex u64 like "D954FB026C21B0DA"; high32 = first 8 hex chars
        if let Ok(hi32) = u32::from_str_radix(&p.id[..8.min(p.id.len())], 16) {
            properties_by_id_hi32.insert(hi32, p);
        }
    }

    GomSchemaDict {
        enums_by_hash,
        classes_by_type_hi32,
        properties_by_id_hi32,
    }
}

/// Resolve a CF40 type marker (the 4-byte high32 of an 8-byte template GUID)
/// to its declared GomProperty.
#[allow(dead_code)]
pub fn property_for_cf40(hi32: u32) -> Option<&'static GomProperty> {
    schema().properties_by_id_hi32.get(&hi32)
}

/// Resolve a class-type hi32 (matches the high32 of an object's template_guid)
/// to its declared GomClass.
#[allow(dead_code)]
pub fn class_for_type_hi32(hi32: u32) -> Option<&'static GomClass> {
    let key = format!("{hi32:08X}");
    schema().classes_by_type_hi32.get(&key)
}

/// Resolve an enum hash (u64) to its declared GomEnum.
#[allow(dead_code)]
pub fn enum_for_hash(hash: u64) -> Option<&'static GomEnum> {
    let key = format!("{hash:016X}");
    schema().enums_by_hash.get(&key)
}

/// Resolve an enum name (e.g. "STAT", "effAction") to its declared GomEnum.
/// Used by the typed-value decoder to look up enum members for enum_ref
/// properties whose tail format is `<05><enum_index_u8>`.
#[allow(dead_code)]
pub fn enum_for_name(name: &str) -> Option<&'static GomEnum> {
    schema().enums_by_hash.values().find(|e| e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_schema() {
        let s = schema();
        assert_eq!(s.enums_by_hash.len(), 748, "enum count drift");
        assert_eq!(s.classes_by_type_hi32.len(), 2220, "class count drift");
        // properties_by_id_hi32 may dedupe a few entries that share high32,
        // so verify >= rather than exact equality.
        assert!(
            s.properties_by_id_hi32.len() >= 9000,
            "property map low: {}",
            s.properties_by_id_hi32.len()
        );
    }

    #[test]
    fn resolves_quest_class() {
        let c = class_for_type_hi32(0x2ADEC3D2).expect("Quest class missing");
        // Agent D's extractor labels root systems like "qst (Quest)"
        let name = c.name.as_deref().unwrap_or("");
        assert!(
            name.contains("qst"),
            "Quest class name unexpected: {name:?}"
        );
        assert!(
            c.property_refs.len() >= 70,
            "Quest property count low: {}",
            c.property_refs.len()
        );
    }

    #[test]
    fn resolves_ability_class() {
        let c = class_for_type_hi32(0x0283F4D2).expect("Ability class missing");
        let name = c.name.as_deref().unwrap_or("");
        assert!(
            name.contains("abl"),
            "Ability class name unexpected: {name:?}"
        );
        assert!(
            c.property_refs.len() >= 40,
            "Ability property count low: {}",
            c.property_refs.len()
        );
    }

    #[test]
    fn resolves_stat_enum() {
        // STAT enum hash D6B68144A4FAFFD7
        let e = enum_for_hash(0xD6B68144A4FAFFD7).expect("STAT enum missing");
        assert!(e.members.len() >= 500, "STAT member count low");
        assert!(
            e.members.iter().any(|m| m.starts_with("STAT_")),
            "STAT enum missing STAT_* members"
        );
    }

    #[test]
    fn resolves_effaction_spvp_members() {
        // effAction enum hash AEA8895FE251D1C7
        let e = enum_for_hash(0xAEA8895FE251D1C7).expect("effAction enum missing");
        // SPVPWeaponDamage is at index 0x95 = 149 per agent A's analysis
        assert_eq!(e.members[0x95], "effAction_SPVPWeaponDamage");
    }

    #[test]
    fn resolves_cf40_d954fb02_stat_selector() {
        let p = property_for_cf40(0xD954FB02).expect("D954FB02 property missing");
        // Decoded per Agent D: D954FB02 selects from the STAT enum
        assert!(
            p.kind == "enum_ref",
            "D954FB02 kind expected enum_ref, got {}",
            p.kind
        );
        if let Some(refs) = &p.refs {
            assert!(
                refs.iter().any(|r| r.name == "STAT"),
                "D954FB02 should resolve to STAT enum, refs = {refs:?}"
            );
        }
    }
}
