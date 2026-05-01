//! Compile-time GSF stat-ID dictionary.
//!
//! Loaded from `gsf_stat_dictionary.toml` and consulted by
//! `populate_gsf_talent_stats`, `populate_gsf_ability_stats`, and
//! `populate_ability_stats` so every row ships with a plain-English `label`
//! and `unit` instead of a raw `stat_id` / `prop_id` byte. Consumers query
//! `WHERE label = 'cooldown_seconds'`, never a hex value.

use anyhow::Result;
use std::collections::HashMap;

const EMBEDDED: &str = include_str!("../gsf_stat_dictionary.toml");

#[derive(Debug, Clone)]
pub struct StatLabel {
    pub label: String,
    pub unit: String,
    pub confidence: String,
}

#[derive(Debug, Default)]
pub struct StatDictionary {
    /// `tal.spvp.*` -- stat_id (u8) -> label
    pub talent_stats: HashMap<u8, StatLabel>,
    /// `abl.spvp.*` -- prop_id (u16) -> label
    pub ability_stats: HashMap<u16, StatLabel>,
    /// `abl.*` ground -- prop_id (u16) -> label
    pub ground_ability_props: HashMap<u16, StatLabel>,
}

impl StatDictionary {
    pub fn from_embedded() -> Result<Self> {
        Self::from_str(EMBEDDED)
    }

    fn from_str(s: &str) -> Result<Self> {
        #[derive(serde::Deserialize)]
        struct File {
            #[serde(default)]
            talent_stats: HashMap<String, Entry>,
            #[serde(default)]
            ability_stats: HashMap<String, Entry>,
            #[serde(default)]
            ground_ability_props: HashMap<String, Entry>,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            label: String,
            unit: String,
            confidence: String,
            #[serde(default)]
            #[allow(dead_code)]
            notes: Option<String>,
        }

        fn parse_hex(s: &str) -> anyhow::Result<u32> {
            let stripped = s.strip_prefix("0x").unwrap_or(s);
            Ok(u32::from_str_radix(stripped, 16)?)
        }

        let file: File = toml::from_str(s)?;

        let mut talent_stats = HashMap::new();
        for (id_hex, entry) in file.talent_stats {
            let id = parse_hex(&id_hex)? as u8;
            talent_stats.insert(
                id,
                StatLabel {
                    label: entry.label,
                    unit: entry.unit,
                    confidence: entry.confidence,
                },
            );
        }

        let mut ability_stats = HashMap::new();
        for (id_hex, entry) in file.ability_stats {
            let id = parse_hex(&id_hex)? as u16;
            ability_stats.insert(
                id,
                StatLabel {
                    label: entry.label,
                    unit: entry.unit,
                    confidence: entry.confidence,
                },
            );
        }

        let mut ground_ability_props = HashMap::new();
        for (id_hex, entry) in file.ground_ability_props {
            let id = parse_hex(&id_hex)? as u16;
            ground_ability_props.insert(
                id,
                StatLabel {
                    label: entry.label,
                    unit: entry.unit,
                    confidence: entry.confidence,
                },
            );
        }

        Ok(Self {
            talent_stats,
            ability_stats,
            ground_ability_props,
        })
    }

    /// Look up a label for a `tal.spvp.*` stat_id. Returns a synthesised
    /// `unknown_0x<id>` label when the byte isn't in the verified table so
    /// the row still ships with a queryable plain-English string.
    pub fn talent_label(&self, stat_id: u8) -> StatLabel {
        self.talent_stats
            .get(&stat_id)
            .cloned()
            .unwrap_or_else(|| StatLabel {
                label: format!("unknown_0x{:02x}", stat_id),
                unit: String::new(),
                confidence: "unknown".to_string(),
            })
    }

    /// Look up a label for an `abl.spvp.*` prop_id.
    pub fn ability_label(&self, prop_id: u16) -> StatLabel {
        self.ability_stats
            .get(&prop_id)
            .cloned()
            .unwrap_or_else(|| StatLabel {
                label: format!("unknown_0x{:04x}", prop_id),
                unit: String::new(),
                confidence: "unknown".to_string(),
            })
    }

    /// Look up a label for an `abl.*` (ground) prop_id.
    pub fn ground_ability_label(&self, prop_id: u16) -> StatLabel {
        self.ground_ability_props
            .get(&prop_id)
            .cloned()
            .unwrap_or_else(|| StatLabel {
                label: format!("unknown_0x{:04x}", prop_id),
                unit: String::new(),
                confidence: "unknown".to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_loads() {
        let dict = StatDictionary::from_embedded().unwrap();
        assert!(dict.talent_stats.contains_key(&0x40));
        assert!(dict.ability_stats.contains_key(&0x0402));
        assert!(dict.ground_ability_props.contains_key(&0x0401));
    }

    #[test]
    fn known_talent_stat_yields_verified_label() {
        let dict = StatDictionary::from_embedded().unwrap();
        let label = dict.talent_label(0x40);
        assert_eq!(label.label, "cooldown_delta_seconds");
        assert_eq!(label.unit, "s");
        assert_eq!(label.confidence, "verified");
    }

    #[test]
    fn unknown_id_yields_synthesised_label() {
        let dict = StatDictionary::from_embedded().unwrap();
        let label = dict.talent_label(0xFE);
        assert_eq!(label.label, "unknown_0xfe");
        assert_eq!(label.confidence, "unknown");
    }
}
