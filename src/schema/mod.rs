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

        // Extract string_id from CE marker after string table type
        let string_id = Self::extract_string_id(&gom.payload);

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

    /// Extract string_id from payload by finding CE marker after string table type.
    /// Pattern: CF 400000115CE87488 (string table type) followed by 02 CE <4-byte LE id>
    /// Valid string IDs are in range 145000-1200000 based on STB extraction.
    fn extract_string_id(payload: &[u8]) -> Option<u32> {
        // String table type marker: CF 40 00 00 11 5C E8 74 88
        const STRING_TABLE_TYPE: [u8; 9] = [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
        const MIN_STRING_ID: u32 = 145_000;
        const MAX_STRING_ID: u32 = 1_200_000;

        // Search for the string table type marker
        for i in 0..payload.len().saturating_sub(STRING_TABLE_TYPE.len() + 6) {
            if payload[i..].starts_with(&STRING_TABLE_TYPE) {
                // After CF + type ID (9 bytes), expect: 02 CE <4-byte LE>
                let after_type = i + STRING_TABLE_TYPE.len();
                if after_type + 6 <= payload.len()
                    && payload[after_type] == 0x02
                    && payload[after_type + 1] == 0xCE
                {
                    let id_bytes = &payload[after_type + 2..after_type + 6];
                    let string_id =
                        u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);

                    // Validate it's in the expected range
                    if (MIN_STRING_ID..=MAX_STRING_ID).contains(&string_id) {
                        return Some(string_id);
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
