//! Schema definitions for SWTOR game objects

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
