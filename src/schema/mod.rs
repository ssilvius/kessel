//! Schema definitions for SWTOR game objects

use crate::pbuk::GomObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Generic game object extracted from GOM
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameObject {
    /// Global unique identifier
    pub guid: String,

    /// Fully qualified name (e.g., "qst.class.warrior.act1.the_hunt")
    pub fqn: String,

    /// Object kind/type (e.g., "Quest", "Ability", "Item", "Npc")
    pub kind: String,

    /// Visual reference / icon name (extracted from payload)
    pub icon_name: Option<String>,

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
                other => other,
            }
        } else {
            "Unknown"
        }.to_string();

        // Extract GUID from header bytes 0-7 (little-endian u64)
        let guid = if gom.header.len() >= 8 {
            let guid_bytes = &gom.header[0..8];
            let guid_u64 = u64::from_le_bytes([
                guid_bytes[0], guid_bytes[1], guid_bytes[2], guid_bytes[3],
                guid_bytes[4], guid_bytes[5], guid_bytes[6], guid_bytes[7],
            ]);
            format!("{:016X}", guid_u64)
        } else {
            String::new()
        };

        // Extract strings from payload for searchability
        let strings = gom.extract_strings();

        // Extract visual reference / icon name from payload
        // Abilities: icon at start, Talents: icon at end
        let icon_name = if gom.fqn.starts_with("tal.") {
            Self::extract_visual_ref_reverse(&gom.payload)
        } else {
            Self::extract_visual_ref(&gom.payload)
        };

        // Encode raw payload as base64 for later analysis
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        let payload_b64 = BASE64.encode(&gom.payload);

        // Store metadata and payload reference in JSON
        let json = serde_json::json!({
            "fqn": gom.fqn,
            "header_hex": hex::encode(&gom.header),
            "payload_size": gom.payload.len(),
            "payload_b64": payload_b64,
            "strings": strings,
        });

        Self {
            guid,
            fqn: gom.fqn.clone(),
            kind,
            icon_name,
            version: 1,
            revision: 1,
            json,
        }
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
                        (b >= b'a' && b <= b'z')
                            || (b >= b'A' && b <= b'Z')
                            || (b >= b'0' && b <= b'9')
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
                        (b >= b'a' && b <= b'z')
                            || (b >= b'A' && b <= b'Z')
                            || (b >= b'0' && b <= b'9')
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

impl GameObject {
    /// Check if this is a quest object
    pub fn is_quest(&self) -> bool {
        self.kind.eq_ignore_ascii_case("quest") || self.fqn.starts_with("qst.")
    }

    /// Check if this is an ability object
    pub fn is_ability(&self) -> bool {
        self.kind.eq_ignore_ascii_case("ability") || self.fqn.starts_with("abl.")
    }

    /// Check if this is an item object
    pub fn is_item(&self) -> bool {
        self.kind.eq_ignore_ascii_case("item") || self.fqn.starts_with("itm.")
    }

    /// Check if this is an NPC object
    pub fn is_npc(&self) -> bool {
        self.kind.eq_ignore_ascii_case("npc") || self.fqn.starts_with("npc.")
    }

    /// Extract localized name from JSON if available
    pub fn name(&self, locale: &str) -> Option<String> {
        // Try common name paths in SWTOR XML
        let paths = [
            vec!["NameList", "Name"],
            vec!["LocalizedName"],
            vec!["Name"],
        ];

        for path in &paths {
            if let Some(name) = self.extract_localized(path, locale) {
                return Some(name);
            }
        }

        None
    }

    fn extract_localized(&self, path: &[&str], locale: &str) -> Option<String> {
        let mut current = &self.json;

        // Navigate to the path
        for &key in path {
            current = current.get(key)?;
        }

        // Look for locale-specific value
        if let Some(localized) = current.get(locale) {
            return localized.as_str().map(|s| s.to_string());
        }

        // Fall back to "enMale" or first available
        if let Some(en) = current.get("enMale").or_else(|| current.get("en")) {
            return en.as_str().map(|s| s.to_string());
        }

        // Try text content
        if let Some(text) = current.get("%") {
            return text.as_str().map(|s| s.to_string());
        }

        None
    }
}
