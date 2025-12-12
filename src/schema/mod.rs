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

        // Store metadata and payload reference in JSON
        let json = serde_json::json!({
            "fqn": gom.fqn,
            "header_hex": hex::encode(&gom.header),
            "payload_size": gom.payload.len(),
            "strings": strings,
        });

        Self {
            guid,
            fqn: gom.fqn.clone(),
            kind,
            version: 1,
            revision: 1,
            json,
        }
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
