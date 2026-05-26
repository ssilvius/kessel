//! Schema definitions for SWTOR game objects

pub mod appearance;
pub mod discipline;
pub mod effect_block;
pub mod epp;
pub mod fxspec;
pub mod gsf_ability;
pub mod gsf_costs;
pub mod gsf_talent;
pub mod item;
pub mod tag_table;

use crate::gom_schema;
use crate::icon_overrides::IconOverrides;
use crate::pbuk::GomObject;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Read 8 little-endian bytes from `header` starting at `offset` and format as
/// 16-char uppercase hex. Returns empty string if the header is too short.
fn read_header_guid(header: &[u8], offset: usize) -> String {
    let end = offset + 8;
    if header.len() < end {
        return String::new();
    }
    let bytes: [u8; 8] = header[offset..end].try_into().unwrap();
    format!("{:016X}", u64::from_le_bytes(bytes))
}

/// Generic game object extracted from GOM
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameObject {
    /// Global unique identifier (from GOM header bytes 0-7)
    pub guid: String,

    /// Kind-level template GUID (from GOM header bytes 16-23, little-endian u64
    /// formatted as 16-char hex). Constant per kind (Quest=c767e4f9..., Npc=bde17800...,
    /// Item=0ecd1a01..., Ability=d2f48302...) with <1% variant outliers. Empirically
    /// verified across 154K objects 2026-04-23.
    pub template_guid: String,

    /// Fully qualified name (e.g., "qst.class.warrior.act1.the_hunt")
    pub fqn: String,

    /// Compound ID: sha256(fqn:guid)[0:16].
    /// Unique per object-instance per extraction. PK in the objects table; the
    /// join key for current-extraction queries. Shifts on patch (because GUID
    /// shifts). For cross-patch identity, use `stable_id` instead.
    pub game_id: String,

    /// FQN-derived ID: sha256(fqn)[0:16].
    /// Stable across patches; unique only post-dedup. Used for cross-version
    /// delta joins.
    pub stable_id: String,

    /// Payload byte hash: sha256(payload_bytes)[0:16].
    /// Not an identity. Change-detector for delta queries:
    /// `WHERE old.payload_hash != new.payload_hash`.
    pub payload_hash: String,

    /// Object kind/type (e.g., "Quest", "Ability", "Item", "Npc")
    pub kind: String,

    /// Visual reference / icon name (extracted from payload, SWTOR's naming)
    pub icon_name: Option<String>,

    /// String table ID (id2) for looking up localized name/description
    /// Extracted from CE marker after CF 400000115CE87488 (string table type)
    pub string_id: Option<u32>,

    /// Schema version
    pub version: u32,

    /// Revision number (for updates)
    pub revision: u32,

    /// Full JSON representation of the object
    pub json: Value,
}

impl GameObject {
    /// Create a GameObject from a GomObject (binary format)
    ///
    /// Since the payload is binary GOM format (not XML), we store:
    /// - FQN directly from the object
    /// - Kind extracted from FQN prefix
    /// - GUID extracted from header bytes (first 8 bytes as hex)
    /// - game_id = sha256(fqn:guid)[0:16] -- unique per extraction; PK
    /// - stable_id = sha256(fqn)[0:16] -- cross-patch identity
    /// - payload_hash = sha256(payload)[0:16] -- delta detector
    /// - Payload stored as base64 in JSON for later parsing
    pub fn from_gom_with_overrides(gom: &GomObject, overrides: Option<&IconOverrides>) -> Self {
        // Extract kind from FQN prefix (e.g., "itm" from "itm.gen.lots...")
        let kind = if let Some(pos) = gom.fqn.find('.') {
            match &gom.fqn[..pos] {
                "qst" => "Quest",
                "mpn" => "Phase",
                "abl" => "Ability",
                "itm" => "Item",
                "npc" => "Npc",
                "cdx" => "Codex",
                "ach" => "Achievement",
                "cnv" => "Conversation",
                "enc" => "Encounter",
                "spn" => "Spawn",
                "plc" => "Placeable",
                "dyn" => "Dynamic",
                "hyd" => "Hydra",
                "tal" => "Talent",
                // Per kessel issue #169: kind labels for the 11 newly-whitelisted
                // PBUK prefixes. Categories per docs/probes/pbuk-prefix-probes.md.
                "dis" => "Discipline",
                "stg" => "Stage",
                "cnd" => "Condition",
                "npp" => "NpcPackage",
                "apn" => "AnimationPackage",
                "cos" => "CosmeticTag",
                "pcs" => "CharacterPreset",
                "nco" => "NpcCompanion",
                "mrp" => "MountPackage",
                "ipp" => "ItemPaintPattern",
                other => other,
            }
        } else {
            "Unknown"
        }
        .to_string();

        // Bytes 0-7: content GUID. Bytes 16-23: kind-level template GUID.
        let guid = read_header_guid(&gom.header, 0);
        let template_guid = read_header_guid(&gom.header, 16);

        // Compute identity columns:
        //   game_id     = sha256(fqn:guid)[0:16] -- unique per extraction (PK)
        //   stable_id   = sha256(fqn)[0:16]      -- cross-patch identity
        //   payload_hash = sha256(payload)[0:16] -- change detector for deltas
        let game_id = crate::hash::compute_game_id(&gom.fqn, &guid);
        let stable_id = crate::hash::compute_stable_id(&gom.fqn);
        let payload_hash = crate::hash::compute_payload_hash(&gom.payload);

        // Extract strings from payload for searchability
        let strings = gom.extract_strings();

        // Extract visual reference / icon name from payload
        // Abilities: icon at start, Talents: icon at end
        // Fall back to compiled-in icon_overrides.toml for abilities whose payloads
        // don't embed the icon reference (e.g. versioned-origin base-class abilities).
        let icon_name = if gom.fqn.starts_with("tal.") {
            Self::extract_visual_ref_reverse(&gom.payload)
        } else {
            Self::extract_visual_ref(&gom.payload)
                .or_else(|| overrides.and_then(|o| o.get(&gom.fqn).map(str::to_string)))
        };

        // Extract string_id: try FQN-based first (finds 91% of quests), then type-marker fallback
        let string_id = Self::extract_string_id_via_fqn_with(&gom.payload, Some(&gom.fqn))
            .or_else(|| Self::extract_string_id_via_type_marker(&gom.payload));

        // Encode raw payload as base64 for later analysis
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let payload_b64 = BASE64.encode(&gom.payload);

        // Store metadata and payload reference in JSON
        let json = serde_json::json!({
            "fqn": gom.fqn,
            "header_hex": hex::encode(&gom.header),
            "payload_size": gom.payload.len(),
            "payload_b64": payload_b64,
            "strings": strings,
            "string_id": string_id,
        });

        Self {
            guid,
            template_guid,
            fqn: gom.fqn.clone(),
            game_id,
            stable_id,
            payload_hash,
            kind,
            icon_name,
            string_id,
            version: 1,
            revision: 1,
            json,
        }
    }

    /// FQN-based string_id extraction.
    fn extract_string_id_via_fqn_with(payload: &[u8], fqn: Option<&str>) -> Option<u32> {
        const MIN_STRING_ID: u32 = 1_000;
        const MAX_STRING_ID: u32 = 10_000_000;

        // Find FQN in payload -- either use provided FQN or scan for dot-separated identifier
        let fqn_end = if let Some(fqn_str) = fqn {
            let fqn_bytes = fqn_str.as_bytes();
            let pos = payload
                .windows(fqn_bytes.len())
                .position(|w| w == fqn_bytes)?;
            pos + fqn_bytes.len()
        } else {
            // Scan for first dot-separated ASCII identifier (the embedded FQN)
            Self::find_embedded_fqn_end(payload)?
        };

        // Scan up to 40 bytes after FQN end for CE marker.
        // The CE marker (3-byte BE string table ID) typically appears 8-20 bytes after the
        // FQN in GOM payloads. 40 bytes provides headroom for objects with extra padding or
        // intermediate fields between FQN and string_id. If CE markers are found beyond this
        // window in practice, increase the limit (extraction validation will show NULL string_id
        // for affected objects).
        let scan_end = (fqn_end + 40).min(payload.len().saturating_sub(3));
        for i in fqn_end..scan_end {
            if payload[i] == 0xCE && i + 4 <= payload.len() {
                // 3-byte big-endian (SWTOR custom CE encoding for string table IDs)
                let stid = (payload[i + 1] as u32) << 16
                    | (payload[i + 2] as u32) << 8
                    | payload[i + 3] as u32;
                if (MIN_STRING_ID..=MAX_STRING_ID).contains(&stid) {
                    return Some(stid);
                }
            }
        }

        None
    }

    /// Find the end position of the first embedded FQN in the payload.
    /// FQNs are dot-separated ASCII identifiers like "qst.class.warrior.act1.the_hunt".
    fn find_embedded_fqn_end(payload: &[u8]) -> Option<usize> {
        // Look for a sequence of ASCII chars with dots (FQN pattern)
        let mut i = 0;
        while i < payload.len().saturating_sub(10) {
            // FQNs start with lowercase ASCII
            if payload[i].is_ascii_lowercase() {
                let start = i;
                let mut has_dot = false;
                let mut j = i;
                while j < payload.len()
                    && (payload[j].is_ascii_lowercase()
                        || payload[j].is_ascii_digit()
                        || payload[j] == b'.'
                        || payload[j] == b'_')
                {
                    if payload[j] == b'.' {
                        has_dot = true;
                    }
                    j += 1;
                }
                let len = j - start;
                // FQNs are at least ~8 chars with dots (e.g., "qst.x.y")
                if has_dot && len >= 8 {
                    return Some(j);
                }
                i = j;
            } else {
                i += 1;
            }
        }
        None
    }

    /// Fallback extraction: search for string table type marker CF 400000115CE87488.
    /// Handles talents and objects where FQN-based extraction fails.
    fn extract_string_id_via_type_marker(payload: &[u8]) -> Option<u32> {
        const STRING_TABLE_TYPE: [u8; 9] = [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
        const MIN_STRING_ID: u32 = 1_000;
        const MAX_STRING_ID: u32 = 10_000_000;

        for i in 0..payload.len().saturating_sub(STRING_TABLE_TYPE.len() + 6) {
            if payload[i..].starts_with(&STRING_TABLE_TYPE) {
                let after_type = i + STRING_TABLE_TYPE.len();
                if after_type + 6 <= payload.len()
                    && payload[after_type] == 0x02
                    && payload[after_type + 1] == 0xCE
                {
                    let id_bytes = &payload[after_type + 2..after_type + 6];

                    // Try 3-byte big-endian first -- the canonical GOM encoding
                    // for string IDs after CE markers (qst, npc, itm, ach, cnv).
                    // A 0x00 separator/flag byte typically follows the 3-byte
                    // ID, which the LE32 decode would incorrectly absorb.
                    let be24 =
                        (id_bytes[0] as u32) << 16 | (id_bytes[1] as u32) << 8 | id_bytes[2] as u32;
                    if (MIN_STRING_ID..=MAX_STRING_ID).contains(&be24) {
                        return Some(be24);
                    }

                    // Fall back to 4-byte little-endian -- discipline talents
                    // and a few other contexts use this. Order swapped from
                    // pre-#37 because LE32 was poisoning achievement IDs.
                    let le32 =
                        u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);
                    if (MIN_STRING_ID..=MAX_STRING_ID).contains(&le32) {
                        return Some(le32);
                    }
                }
            }
        }

        None
    }

    /// Extract visual reference / icon name from payload.
    /// Looks for pattern: 0x06 <length> <ascii_string> in first 60 bytes.
    fn extract_visual_ref(payload: &[u8]) -> Option<String> {
        let search_limit = payload.len().min(60);

        for i in 0..search_limit.saturating_sub(4) {
            if payload[i] == 0x06 {
                let length = payload[i + 1] as usize;
                if length > 4 && length < 60 && i + 2 + length <= payload.len() {
                    let potential = &payload[i + 2..i + 2 + length];
                    // Check if ASCII alphanumeric with underscores
                    if potential.iter().all(|&b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_uppercase()
                            || b.is_ascii_digit()
                            || b == b'_'
                    }) {
                        if let Ok(s) = std::str::from_utf8(potential) {
                            // Must contain underscore or be purely alphabetic
                            if s.contains('_') || s.chars().all(|c| c.is_alphabetic()) {
                                return Some(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract visual reference from end of payload (for talents).
    /// Searches backwards from the last 100 bytes.
    fn extract_visual_ref_reverse(payload: &[u8]) -> Option<String> {
        if payload.len() < 10 {
            return None;
        }

        // Search the last 100 bytes, backwards
        let start = payload.len().saturating_sub(100);
        let mut last_match: Option<String> = None;

        for i in start..payload.len().saturating_sub(4) {
            if payload[i] == 0x06 {
                let length = payload[i + 1] as usize;
                if length > 4 && length < 60 && i + 2 + length <= payload.len() {
                    let potential = &payload[i + 2..i + 2 + length];
                    // Check if ASCII alphanumeric with underscores
                    if potential.iter().all(|&b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_uppercase()
                            || b.is_ascii_digit()
                            || b == b'_'
                    }) {
                        if let Ok(s) = std::str::from_utf8(potential) {
                            // Must contain underscore or be purely alphabetic
                            // Skip "str.tal" prefix strings
                            if !s.starts_with("str.")
                                && (s.contains('_') || s.chars().all(|c| c.is_alphabetic()))
                            {
                                last_match = Some(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        last_match
    }
}

// ----- Schema-aware payload walker (#125) ------------------------------------
//
// Walks a GOM payload and resolves CF40 type markers via the gom_schema
// dictionary to produce typed, named properties alongside the existing raw
// hex output. Additive: callers that read the existing JSON output continue to
// work; consumers who want typed access read the new `named_props` field.

/// Result of a schema-aware payload decode.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct SchemaAwareDecoded {
    /// Existing hex-keyed format -- one entry per CF40 marker found.
    /// Example: `{"D954FB02": null}` (raw key, value placeholder).
    pub raw_props: Value,

    /// Named typed properties resolved via gom_schema. Keys are the property
    /// kind ("enum_ref", "int32", etc) combined with the resolved target name
    /// when available (e.g. "stat_selector").
    pub named_props: Value,

    /// Class name resolved from the input `class_type_hi32`, if any.
    pub class_name: Option<String>,

    /// Number of CF40 markers in the payload that resolved to a known property.
    pub property_count_resolved: usize,

    /// Number of CF40 markers in the payload that did NOT resolve.
    pub property_count_unresolved: usize,
}

/// Walk a GOM payload, resolve CF40 type markers via `gom_schema`, and emit
/// typed properties alongside the existing raw hex output.
///
/// `class_type_hi32` is the high32 of the object's `template_guid` (per the
/// existing `GameObject::template_guid` formatting -- take the leading 8 hex
/// characters and parse as u32).
///
/// This is an ADDITIVE function: it produces a new `SchemaAwareDecoded` struct
/// without modifying any existing output. Consumers that don't call this see
/// no change in their data.
#[allow(dead_code)]
pub fn decode_payload_schema_aware(
    payload: &[u8],
    class_type_hi32: u32,
) -> Result<SchemaAwareDecoded> {
    let mut raw_props = serde_json::Map::new();
    let mut named_props = serde_json::Map::new();
    let mut resolved = 0usize;
    let mut unresolved = 0usize;

    // Scan for CF 40 00 00 markers (template_guid prefix used by GOM payloads).
    // Wire format per docs/probes/typed-value-encoding.md (verified across
    // 9.3M markers in 415K canonical objects):
    //
    //   byte 0..4   CF 40 00 00
    //   byte 4      type_flags (per-property stable; not a separate decode)
    //   byte 5..9   hi32 BE (property id high32; key for gom_schema lookup)
    //   byte 9      tail[0] = on-wire type tag (NOT always == schema.kind label)
    //   byte 10..   value bytes per type tag
    //
    // Type tags actually observed (verified-from-payloads names, NOT the
    // mislabeled gom_schema.kind strings):
    //   0x01, 0x07 = wrapper (advance 1 byte and recurse into inner type)
    //   0x02, 0x03 = int8 (1-byte signed value)
    //   0x04       = float32 (4 LE bytes; schema labels this "int32" -- WRONG)
    //   0x05       = container (per-property element grammar; here we record
    //                the byte after 0x05 either as enum index resolved against
    //                the property's refs, or as a list-count)
    //   0x06       = length-prefixed string (1 len byte + N ASCII bytes;
    //                schema labels this "float32" -- WRONG)
    //   0x08       = array (per-property element grammar; here we record the
    //                element-type byte and stop)
    //   0x09       = class_ref via embedded CF E0 GUID (12 bytes)
    //   0x12       = Vec3 of 3 LE float32 (13 bytes)
    //   0x0E/0x11/0x14/0x15 = never appear in payloads (schema-declared only)
    let mut i = 0;
    while i + 9 <= payload.len() {
        if payload[i] != 0xCF
            || payload[i + 1] != 0x40
            || payload[i + 2] != 0x00
            || payload[i + 3] != 0x00
        {
            i += 1;
            continue;
        }
        let hi32_bytes: [u8; 4] = payload[i + 5..i + 9].try_into().unwrap();
        let hi32 = u32::from_be_bytes(hi32_bytes);
        let raw_key = format!("{hi32:08X}");

        let (decoded_value, _consumed) = decode_value_tail(payload, i + 9, hi32, 0);
        raw_props.insert(raw_key.clone(), decoded_value.clone());

        match gom_schema::property_for_cf40(hi32) {
            Some(prop) => {
                resolved += 1;
                // Key by hi32 so same-kind properties don't collide.
                // Format: `kind__HI32[__enum_name]` -- consumers can match
                // by substring (e.g. "qstActivityType") and still get a
                // unique value per property occurrence.
                let mut named_key = format!("{}__{raw_key}", prop.kind);
                if let Some(refs) = &prop.refs {
                    if let Some(first) = refs.first() {
                        named_key = format!("{}__{raw_key}__{}", prop.kind, first.name);
                    }
                }
                named_props.insert(named_key, decoded_value);
            }
            None => {
                unresolved += 1;
            }
        }

        i += 9;
    }

    let class_name = gom_schema::class_for_type_hi32(class_type_hi32).and_then(|c| c.name.clone());

    Ok(SchemaAwareDecoded {
        raw_props: Value::Object(raw_props),
        named_props: Value::Object(named_props),
        class_name,
        property_count_resolved: resolved,
        property_count_unresolved: unresolved,
    })
}

/// Decode one typed value tail starting at `payload[pos]`. Returns the
/// decoded value as a `serde_json::Value` and the number of bytes consumed
/// from `pos` (inclusive of the type tag byte). `hi32` is the parent CF40
/// marker's property id, used for enum-name resolution on tag 0x05.
///
/// Encoding table per docs/probes/typed-value-encoding.md. Mismatched
/// schema-dictionary labels are handled correctly here (we go by tail[0],
/// not by the dictionary's `kind` field which is mislabeled for 5 of 10
/// tags).
fn decode_value_tail(payload: &[u8], pos: usize, hi32: u32, depth: u8) -> (Value, usize) {
    use serde_json::json;
    if pos >= payload.len() {
        return (Value::Null, 0);
    }
    let tag = payload[pos];
    // Bound recursion on wrappers
    if depth > 4 {
        return (json!({"deep_wrap": format!("0x{tag:02X}")}), 1);
    }
    match tag {
        // Wrappers: advance 1 byte and recurse into the inner type.
        0x01 | 0x07 => {
            let (inner, n) = decode_value_tail(payload, pos + 1, hi32, depth + 1);
            (json!({"wrap": format!("0x{tag:02X}"), "v": inner}), 1 + n)
        }
        // int8 (both 0x02 and 0x03 -- schema mislabels 0x03 as "int16")
        0x02 | 0x03 => {
            if pos + 1 >= payload.len() {
                return (Value::Null, 1);
            }
            (json!(payload[pos + 1] as i8), 2)
        }
        // float32 (schema mislabels this as "int32")
        0x04 => {
            if pos + 5 > payload.len() {
                return (Value::Null, 1);
            }
            let bytes: [u8; 4] = payload[pos + 1..pos + 5].try_into().unwrap();
            let f = f32::from_le_bytes(bytes);
            if f.is_finite() {
                (json!(f as f64), 5)
            } else {
                (Value::Null, 5)
            }
        }
        // Container / enum-or-list. For enum_ref-typed properties the byte
        // after 0x05 is the enum index; resolve against the property's
        // first ref name. For non-enum properties it's a list count.
        0x05 => {
            if pos + 1 >= payload.len() {
                return (Value::Null, 1);
            }
            let idx_or_count = payload[pos + 1];
            // Resolve enum name if the property declares one
            if let Some(prop) = gom_schema::property_for_cf40(hi32) {
                if let Some(refs) = &prop.refs {
                    if let Some(first) = refs.first() {
                        if first.kind == "enum" {
                            if let Some(e) = gom_schema::enum_for_name(&first.name) {
                                if (idx_or_count as usize) < e.members.len() {
                                    return (
                                        json!({
                                            "enum": &first.name,
                                            "index": idx_or_count,
                                            "name": &e.members[idx_or_count as usize],
                                        }),
                                        2,
                                    );
                                }
                            }
                            return (json!({"enum": &first.name, "index": idx_or_count}), 2);
                        }
                    }
                }
            }
            (json!({"list_count": idx_or_count}), 2)
        }
        // Length-prefixed string (schema mislabels this as "float32")
        0x06 => {
            if pos + 1 >= payload.len() {
                return (Value::Null, 1);
            }
            let len = payload[pos + 1] as usize;
            if len == 0 || pos + 2 + len > payload.len() {
                return (Value::Null, 2);
            }
            let bytes = &payload[pos + 2..pos + 2 + len];
            if bytes.iter().all(|&b| (0x20..0x7F).contains(&b)) {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    return (json!(s), 2 + len);
                }
            }
            (Value::Null, 2 + len)
        }
        // Array opener with per-property element grammar. Record the
        // element type byte; per-property full decode lives in consumers.
        0x08 => {
            let elem_type = if pos + 1 < payload.len() {
                Some(payload[pos + 1])
            } else {
                None
            };
            (
                json!({"array_element_type": elem_type.map(|b| format!("0x{b:02X}"))}),
                1,
            )
        }
        // class_ref via embedded CF E0 GUID (12 bytes total: 09 02 00 CF E0 00 + 6-byte tail).
        0x09 => {
            if pos + 12 > payload.len() {
                return (Value::Null, 1);
            }
            // Confirm the embedded CF E0 marker shape before pulling the GUID
            if payload[pos + 3] == 0xCF && payload[pos + 4] == 0xE0 {
                let tail = &payload[pos + 6..pos + 12];
                let guid = format!("E000{}", hex::encode_upper(tail));
                return (json!({"class_ref_guid": guid}), 12);
            }
            (
                json!({"class_ref_raw": hex::encode_upper(&payload[pos + 1..pos + 12])}),
                12,
            )
        }
        // Vec3 of 3 LE float32 (1 tag + 12 value = 13 bytes)
        0x12 => {
            if pos + 13 > payload.len() {
                return (Value::Null, 1);
            }
            let x = f32::from_le_bytes(payload[pos + 1..pos + 5].try_into().unwrap());
            let y = f32::from_le_bytes(payload[pos + 5..pos + 9].try_into().unwrap());
            let z = f32::from_le_bytes(payload[pos + 9..pos + 13].try_into().unwrap());
            (json!([x as f64, y as f64, z as f64]), 13)
        }
        // Tags 0x0E/0x11/0x14/0x15 do not appear in payloads per the
        // 9.3M-marker scan; anything else is unrecognized.
        _ => (json!({"unknown_tag": format!("0x{tag:02X}")}), 1),
    }
}

#[cfg(test)]
mod schema_walker_tests {
    use super::*;

    fn build_cf40_marker(hi32: u32) -> [u8; 9] {
        // Real wire format: CF 40 00 00 [type_byte] [hi32 BE 4 bytes].
        // type_byte is non-zero in real payloads (0x11/0x13/0x40 observed);
        // use 0x40 here as a generic stand-in.
        let mut buf = [0u8; 9];
        buf[0] = 0xCF;
        buf[1] = 0x40;
        buf[4] = 0x40;
        buf[5..9].copy_from_slice(&hi32.to_be_bytes());
        buf
    }

    #[test]
    fn decodes_known_cf40_marker_resolves() {
        // CF40 D954FB02 = STAT selector per Agent D (verified in #124).
        let payload = build_cf40_marker(0xD954FB02);
        let d = decode_payload_schema_aware(&payload, 0xD954FB01).expect("decode");
        assert!(d.property_count_resolved >= 1, "D954FB02 should resolve");
        let raw = d.raw_props.as_object().unwrap();
        assert!(raw.contains_key("D954FB02"), "raw_props key missing");
        let named = d.named_props.as_object().unwrap();
        // Named key should mention STAT (the resolved enum target).
        assert!(
            named.keys().any(|k| k.contains("STAT")),
            "named_props missing STAT: {named:?}"
        );
    }

    #[test]
    fn unresolved_marker_counts() {
        // CF40 DEADBEEF is not a known property hi32 -> unresolved.
        let payload = build_cf40_marker(0xDEADBEEF);
        let d = decode_payload_schema_aware(&payload, 0xD954FB01).expect("decode");
        assert_eq!(d.property_count_unresolved, 1);
        let raw = d.raw_props.as_object().unwrap();
        assert!(raw.contains_key("DEADBEEF"));
    }

    #[test]
    fn resolves_class_name_for_quest() {
        let payload = []; // empty payload OK for class lookup
        let d = decode_payload_schema_aware(&payload, 0x2ADEC3D2).expect("decode");
        let name = d.class_name.as_deref().unwrap_or("");
        assert!(
            name.contains("qst"),
            "Quest class name unexpected: {name:?}"
        );
    }

    #[test]
    fn empty_payload_no_markers() {
        let d = decode_payload_schema_aware(&[], 0xD954FB01).expect("decode");
        assert_eq!(d.property_count_resolved, 0);
        assert_eq!(d.property_count_unresolved, 0);
    }

    #[test]
    fn decodes_real_talent_payload_markers() {
        // Real `tal.sith_inquisitor.skill.electric_induction` payload prefix.
        // Markers: A787EE87 at +2 (effect-block array), 5CE87488 at +27 (int8).
        let payload: [u8; 88] = [
            0x08, 0x04, 0xCF, 0x40, 0x00, 0x00, 0x13, 0xA7, 0x87, 0xEE, 0x87, 0x08, 0x02, 0x09,
            0x02, 0x02, 0xCF, 0x26, 0xF1, 0xAA, 0xD9, 0xFC, 0xA9, 0xED, 0x09, 0x04, 0x04, 0xCF,
            0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88, 0x02, 0xCE, 0x0C, 0x1E, 0xD6, 0x00,
            0x00, 0x00, 0x01, 0x01, 0x06, 0x07, 0x73, 0x74, 0x72, 0x2E, 0x74, 0x61, 0x6C, 0xCC,
            0x37, 0xAE, 0x6F, 0x6F, 0xE7, 0x08, 0x05, 0x04, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x08, 0x05, 0x06, 0x00, 0x00, 0xCF, 0xD9, 0xAD, 0xAE, 0xC5, 0xF2, 0x75,
            0xD8, 0x46, 0x04, 0x04,
        ];
        // Talent class type_hi32 = 0xD954FB01 per gom_schema 019e4d75.
        let d = decode_payload_schema_aware(&payload, 0xD954FB01).expect("decode");
        let raw = d.raw_props.as_object().unwrap();
        assert!(
            raw.contains_key("A787EE87"),
            "A787EE87 (effect-block array) should resolve: {raw:?}"
        );
        assert!(
            raw.contains_key("5CE87488"),
            "5CE87488 (int8) should resolve: {raw:?}"
        );
        assert!(
            d.property_count_resolved >= 2,
            "expected >=2 resolved markers, got {}",
            d.property_count_resolved
        );
    }

    #[test]
    fn additive_does_not_break_existing_decoder() {
        // Regression guard: the new walker must not modify any existing API
        // path. Verify the existing GameObject::from_gom_with_overrides remains
        // callable.
        let dummy_gom = GomObject {
            fqn: "test.fqn".to_string(),
            header: vec![0u8; 42],
            payload: vec![],
        };
        let _obj = GameObject::from_gom_with_overrides(&dummy_gom, None);
        // If this compiles + the test infrastructure runs, the additive API
        // hasn't disturbed existing call sites.
    }
}
