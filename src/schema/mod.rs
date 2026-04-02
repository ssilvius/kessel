//! Schema definitions for SWTOR game objects

use crate::pbuk::GomObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Generic game object extracted from GOM
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameObject {
    /// Global unique identifier (from GOM header)
    pub guid: String,

    /// Fully qualified name (e.g., "qst.class.warrior.act1.the_hunt")
    pub fqn: String,

    /// Compound ID: sha256(fqn:guid)[0:16] - deterministic, collision-resistant
    /// Used for consistent naming of all related assets (icons, etc.)
    pub game_id: String,

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
    /// - game_id = sha256(fqn:guid)[0:16] for consistent asset naming
    /// - Payload stored as base64 in JSON for later parsing
    pub fn from_gom(gom: &GomObject) -> Self {
        // Extract kind from FQN prefix (e.g., "itm" from "itm.gen.lots...")
        let kind = if let Some(pos) = gom.fqn.find('.') {
            match &gom.fqn[..pos] {
                "qst" | "mpn" => "Quest",
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
                other => other,
            }
        } else {
            "Unknown"
        }
        .to_string();

        // Extract GUID from header bytes 0-7 (little-endian u64)
        let guid = if gom.header.len() >= 8 {
            let guid_bytes = &gom.header[0..8];
            let guid_u64 = u64::from_le_bytes([
                guid_bytes[0],
                guid_bytes[1],
                guid_bytes[2],
                guid_bytes[3],
                guid_bytes[4],
                guid_bytes[5],
                guid_bytes[6],
                guid_bytes[7],
            ]);
            format!("{:016X}", guid_u64)
        } else {
            String::new()
        };

        // Compute game_id: sha256(fqn:guid)[0:16] - deterministic compound ID
        let game_id = crate::hash::compute_game_id(&gom.fqn, &guid);

        // Extract strings from payload for searchability
        let strings = gom.extract_strings();

        // Extract visual reference / icon name from payload
        // Abilities: icon at start, Talents: icon at end
        let icon_name = if gom.fqn.starts_with("tal.") {
            Self::extract_visual_ref_reverse(&gom.payload)
        } else {
            Self::extract_visual_ref(&gom.payload)
        };

        // Extract string_id: try FQN-based first (finds 91% of quests), then type-marker fallback
        let string_id =
            Self::extract_string_id_via_fqn_with(&gom.payload, Some(&gom.fqn))
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
            fqn: gom.fqn.clone(),
            game_id,
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
        const STRING_TABLE_TYPE: [u8; 9] =
            [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
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

                    // Try 4-byte little-endian first (discipline talents)
                    let le32 =
                        u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);
                    if (MIN_STRING_ID..=MAX_STRING_ID).contains(&le32) {
                        return Some(le32);
                    }

                    // Fall back to 3-byte big-endian (GSF talents: tal.spvp.*)
                    let be24 = (id_bytes[0] as u32) << 16
                        | (id_bytes[1] as u32) << 8
                        | id_bytes[2] as u32;
                    if (MIN_STRING_ID..=MAX_STRING_ID).contains(&be24) {
                        return Some(be24);
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
