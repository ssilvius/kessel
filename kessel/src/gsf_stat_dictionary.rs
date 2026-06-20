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
    /// FQN-prefix overrides for `tal.spvp.*` stat_ids whose meaning depends on
    /// the parent talent (e.g. 0x40 is normally `cooldown_delta_seconds` but
    /// `minor_sensors.com_range.*` repurposes it as `comm_range_units`).
    pub talent_stat_overrides: Vec<TalentStatOverride>,
    /// FQN-prefix overrides for `abl.spvp.*` prop_ids whose meaning depends on
    /// the ability (e.g. 0x0403 is a vacuous `scaling_factor` default for most
    /// abilities but a real `weapon_cooldown_seconds` on cooldown-gated drone
    /// weapons). #301.
    pub ability_stat_overrides: Vec<AbilityStatOverride>,
}

#[derive(Debug, Clone)]
pub struct TalentStatOverride {
    pub fqn_prefix: String,
    pub stat_id: u8,
    pub label: StatLabel,
}

#[derive(Debug, Clone)]
pub struct AbilityStatOverride {
    pub fqn_prefix: String,
    pub prop_id: u16,
    pub label: StatLabel,
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
            #[serde(default)]
            talent_stat_overrides: Vec<OverrideEntry>,
            #[serde(default)]
            ability_stat_overrides: Vec<AbilityOverrideEntry>,
        }
        #[derive(serde::Deserialize)]
        struct AbilityOverrideEntry {
            fqn_prefix: String,
            prop_id: String,
            label: String,
            unit: String,
            confidence: String,
            #[serde(default)]
            #[allow(dead_code)]
            notes: Option<String>,
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
        #[derive(serde::Deserialize)]
        struct OverrideEntry {
            fqn_prefix: String,
            stat_id: String,
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

        let mut talent_stat_overrides = Vec::with_capacity(file.talent_stat_overrides.len());
        for entry in file.talent_stat_overrides {
            let stat_id = parse_hex(&entry.stat_id)? as u8;
            talent_stat_overrides.push(TalentStatOverride {
                fqn_prefix: entry.fqn_prefix,
                stat_id,
                label: StatLabel {
                    label: entry.label,
                    unit: entry.unit,
                    confidence: entry.confidence,
                },
            });
        }
        // Longest prefix first so a more specific override wins over a shorter
        // sibling if any future entries overlap.
        talent_stat_overrides.sort_by_key(|ov| std::cmp::Reverse(ov.fqn_prefix.len()));

        let mut ability_stat_overrides = Vec::with_capacity(file.ability_stat_overrides.len());
        for entry in file.ability_stat_overrides {
            let prop_id = parse_hex(&entry.prop_id)? as u16;
            ability_stat_overrides.push(AbilityStatOverride {
                fqn_prefix: entry.fqn_prefix,
                prop_id,
                label: StatLabel {
                    label: entry.label,
                    unit: entry.unit,
                    confidence: entry.confidence,
                },
            });
        }
        ability_stat_overrides.sort_by_key(|ov| std::cmp::Reverse(ov.fqn_prefix.len()));

        Ok(Self {
            talent_stats,
            ability_stats,
            ground_ability_props,
            talent_stat_overrides,
            ability_stat_overrides,
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

    /// FQN-aware label lookup for `tal.spvp.*`. Consults `talent_stat_overrides`
    /// first; falls back to the default `talent_label` mapping. Use this from
    /// `populate_gsf_talent_stats` so context-overloaded stat_ids (0x40 acting
    /// as comm range on minor_sensors.com_range.*) ship with the right label.
    pub fn talent_label_for(&self, stat_id: u8, fqn: &str) -> StatLabel {
        for ov in &self.talent_stat_overrides {
            if ov.stat_id == stat_id && fqn.starts_with(&ov.fqn_prefix) {
                return ov.label.clone();
            }
        }
        self.talent_label(stat_id)
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

    /// FQN-aware label lookup for `abl.spvp.*`. Consults `ability_stat_overrides`
    /// first (so a prop_id whose meaning is FQN-dependent -- e.g. 0x0403 acting
    /// as a real weapon cooldown on drone weapons rather than the vacuous
    /// scaling_factor default -- ships with the right label) then falls back to
    /// the default `ability_label` mapping. #301.
    pub fn ability_label_for(&self, prop_id: u16, fqn: &str) -> StatLabel {
        for ov in &self.ability_stat_overrides {
            if ov.prop_id == prop_id && fqn.starts_with(&ov.fqn_prefix) {
                return ov.label.clone();
            }
        }
        self.ability_label(prop_id)
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
    fn ability_0x0403_override_only_on_drone_weapons() {
        let dict = StatDictionary::from_embedded().unwrap();
        // Drone weapons: 0x0403 is the weapon cooldown.
        assert_eq!(
            dict.ability_label_for(0x0403, "abl.spvp.drone.sentry_sniper.railgun")
                .label,
            "weapon_cooldown_seconds"
        );
        assert_eq!(
            dict.ability_label_for(0x0403, "abl.spvp.drone.sentry_missile.missile")
                .label,
            "weapon_cooldown_seconds"
        );
        // Everything else keeps the default vacuous scaling_factor.
        assert_eq!(
            dict.ability_label_for(0x0403, "abl.spvp.shield.distortion_field")
                .label,
            "scaling_factor"
        );
        // A non-overridden prop_id is unaffected by FQN.
        assert_eq!(
            dict.ability_label_for(0x0402, "abl.spvp.drone.sentry_sniper.railgun")
                .label,
            "cooldown_seconds"
        );
    }

    #[test]
    fn unknown_id_yields_synthesised_label() {
        let dict = StatDictionary::from_embedded().unwrap();
        let label = dict.talent_label(0xFE);
        assert_eq!(label.label, "unknown_0xfe");
        assert_eq!(label.confidence, "unknown");
    }

    #[test]
    fn fqn_override_relabels_collision_stats() {
        let dict = StatDictionary::from_embedded().unwrap();
        // Default for 0x40 is cooldown_delta_seconds.
        let default_label = dict.talent_label_for(0x40, "tal.spvp.shield.shield_projector.tier1");
        assert_eq!(default_label.label, "cooldown_delta_seconds");
        assert_eq!(default_label.unit, "s");

        // minor_sensors.com_range.* repurposes 0x40 as comm_range_units.
        let overridden = dict.talent_label_for(0x40, "tal.spvp.minor_sensors.com_range.base");
        assert_eq!(overridden.label, "comm_range_units");
        assert_eq!(overridden.unit, "units");
        assert_eq!(overridden.confidence, "verified");

        // crew.tactical.communications_range is also overridden.
        let comm_boost = dict.talent_label_for(0x40, "tal.spvp.crew.tactical.communications_range");
        assert_eq!(comm_boost.label, "comm_range_units");

        // crew.tactical.sensor_volume reuses 0x41 as sensor_volume_units.
        let sensor_volume = dict.talent_label_for(0x41, "tal.spvp.crew.tactical.sensor_volume");
        assert_eq!(sensor_volume.label, "sensor_volume_units");
        assert_eq!(sensor_volume.unit, "units");
    }

    #[test]
    fn new_decoded_labels_present() {
        let dict = StatDictionary::from_embedded().unwrap();
        // Spot-check the labels huttspawn needs for the build calculator.
        for (id, expected) in [
            (0x46u8, "shield_power_pool_pct"),
            (0x45, "weapon_power_pool_pct"),
            (0x08, "hull_strength_pct"),
            (0x58, "damage_reduction_pct"),
            (0x38, "evasion_pct"),
            (0x4e, "accuracy_pct"),
            (0x3f, "sensor_range_units"),
            (0x44, "sensor_dampening_units"),
            (0x55, "shield_regen_delay_pct"),
            (0x20, "shield_power_regen_pct"),
            (0x1e, "weapon_power_regen_pct"),
        ] {
            let label = dict.talent_label(id);
            assert_eq!(label.label, expected, "stat 0x{:02x}", id);
            assert_eq!(label.confidence, "verified", "stat 0x{:02x}", id);
        }
    }
}
