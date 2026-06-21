//! Description-anchoring parser (#323, epic #322): extract typed numeric facts
//! from a localized description string.
//!
//! The displayed numeric values for items/abilities/talents/GSF/missions are
//! literals in the localized strings -- a number the game shows the player in
//! EN/FR/DE is string-table data by construction. This module parses those
//! literals (and `<<N>>` template ordinals) into typed [`AnchoredFact`]s; the
//! populate pass (#324) and the per-domain promotions (#325/#326) anchor them
//! to records. Pure and deterministic -- no DB, no I/O.
//!
//! Enabler module: the public API below is consumed by the `description_values`
//! populate pass (#324). Until that lands there is no in-crate caller, so the
//! binary target would flag the API as dead -- allow it here and drop the allow
//! when #324 wires the consumer.
#![allow(dead_code)]

use regex::Regex;
use std::sync::OnceLock;

/// A typed numeric fact parsed from a description string.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchoredFact {
    pub kind: FactKind,
    /// Literal value normalized to a base unit (seconds, meters, points; the raw
    /// percent number for [`FactKind::Percent`]). `0.0` for [`FactKind::Template`].
    pub value: f64,
    /// The stat/unit word attached to the number (`power`, `seconds`, `chance`,
    /// `critical rating`), lowercased.
    pub label: String,
    /// Byte offset of the match start in the source string (for ordering/dedup).
    pub at: usize,
}

/// The classification of a parsed numeric fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactKind {
    /// `30%`
    Percent,
    /// `20 seconds`, `5 minutes` -- normalized to seconds.
    DurationSeconds,
    /// `8 meter(s)`, `5000m`.
    RangeMeters,
    /// `4 targets`, `3 stacks`.
    Count,
    /// `510 Power`, `Critical Rating by 40`.
    Magnitude,
    /// `<<N>>` -- value filled at runtime (level-scaled damage/heal) or
    /// recoverable via the `payload_ordinal` substrate (durations).
    Template(u32),
}

/// Normalize a duration unit word to a seconds multiplier.
fn duration_mult(word: &str) -> Option<f64> {
    match word {
        "second" | "seconds" | "s" => Some(1.0),
        "minute" | "minutes" => Some(60.0),
        "hour" | "hours" => Some(3600.0),
        _ => None,
    }
}

fn is_range(w: &str) -> bool {
    matches!(w, "meter" | "meters" | "m")
}

fn is_count(w: &str) -> bool {
    matches!(
        w,
        "target"
            | "targets"
            | "enemy"
            | "enemies"
            | "times"
            | "charge"
            | "charges"
            | "stack"
            | "stacks"
            | "ally"
            | "allies"
    )
}

fn is_stat(w: &str) -> bool {
    matches!(
        w,
        "power"
            | "mastery"
            | "defense"
            | "critical"
            | "alacrity"
            | "accuracy"
            | "absorption"
            | "shield"
            | "endurance"
            | "presence"
            | "willpower"
            | "cunning"
            | "aim"
            | "strength"
            | "health"
            | "energy"
            | "focus"
            | "rage"
            | "force"
    )
}

/// `<<N>>` (opt `[..]` format spec) | `N%` | `N word`. Compiled once.
fn num_unit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:<<(\d+)(?:\[[^\]]*\])?>>|(\d+(?:\.\d+)?)\s*%|(\d+(?:\.\d+)?))\s*([A-Za-z]+)?",
        )
        .expect("static num_unit regex is valid")
    })
}

/// The stat-BEFORE-number orientation: "increases X (Rating) by N(%)".
fn stat_phrase_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:increase[sd]?|reduce[sd]?|by)\s+(?:your\s+)?([A-Z][A-Za-z ]{2,24}?)\s+(?:Rating\s+)?by\s+(\d+(?:\.\d+)?)\s*(%?)",
        )
        .expect("static stat_phrase regex is valid")
    })
}

/// Parse every numeric fact from `text`. Total function -- never panics; numbers
/// that fail to parse are skipped, and text with no facts yields an empty Vec.
pub fn parse_description(text: &str) -> Vec<AnchoredFact> {
    let mut facts = Vec::new();

    // stat-before-number ("Increases Critical Rating by 40")
    for c in stat_phrase_re().captures_iter(text) {
        let m = c.get(0).expect("group 0 always present");
        let Some(v) = c.get(2).and_then(|g| g.as_str().parse::<f64>().ok()) else {
            continue;
        };
        let pct = c.get(3).is_some_and(|g| !g.as_str().is_empty());
        facts.push(AnchoredFact {
            kind: if pct {
                FactKind::Percent
            } else {
                FactKind::Magnitude
            },
            value: v,
            label: c[1].trim().to_lowercase(),
            at: m.start(),
        });
    }

    // number-with-unit / percent / template
    for c in num_unit_re().captures_iter(text) {
        let at = c.get(0).expect("group 0 always present").start();
        let word = c
            .get(4)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        if let Some(ord) = c.get(1).and_then(|g| g.as_str().parse::<u32>().ok()) {
            facts.push(AnchoredFact {
                kind: FactKind::Template(ord),
                value: 0.0,
                label: word,
                at,
            });
        } else if let Some(p) = c.get(2).and_then(|g| g.as_str().parse::<f64>().ok()) {
            facts.push(AnchoredFact {
                kind: FactKind::Percent,
                value: p,
                label: if word.is_empty() { "pct".into() } else { word },
                at,
            });
        } else if let Some(v) = c.get(3).and_then(|g| g.as_str().parse::<f64>().ok()) {
            let kind = if let Some(mult) = duration_mult(&word) {
                Some((FactKind::DurationSeconds, v * mult))
            } else if is_range(&word) {
                Some((FactKind::RangeMeters, v))
            } else if is_count(&word) {
                Some((FactKind::Count, v))
            } else if is_stat(&word) {
                Some((FactKind::Magnitude, v))
            } else {
                None // bare number with a non-unit word -> flavor/noise; skip
            };
            if let Some((kind, value)) = kind {
                facts.push(AnchoredFact {
                    kind,
                    value,
                    label: word,
                    at,
                });
            }
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_percent_duration_and_template() {
        let f = parse_description(
            "30% chance to grant 510 Power for <<1>> seconds. This effect can only occur once every 20 seconds.",
        );
        assert!(f
            .iter()
            .any(|x| x.kind == FactKind::Percent && x.value == 30.0));
        assert!(f
            .iter()
            .any(|x| x.kind == FactKind::DurationSeconds && x.value == 20.0));
        assert!(f.iter().any(|x| x.kind == FactKind::Template(1)));
    }

    #[test]
    fn parses_count_and_range() {
        let f = parse_description("stealing health from up to 4 targets within an 8 meter radius");
        assert!(f
            .iter()
            .any(|x| x.kind == FactKind::Count && x.value == 4.0));
        assert!(f
            .iter()
            .any(|x| x.kind == FactKind::RangeMeters && x.value == 8.0));
    }

    #[test]
    fn parses_both_stat_orientations() {
        assert!(parse_description("grant 510 Power")
            .iter()
            .any(|x| x.kind == FactKind::Magnitude && x.value == 510.0 && x.label == "power"));
        assert!(parse_description("Increases Critical Rating by 40")
            .iter()
            .any(|x| x.kind == FactKind::Magnitude
                && x.value == 40.0
                && x.label.contains("critical")));
    }

    #[test]
    fn normalizes_minutes_to_seconds() {
        assert!(parse_description("lasts 5 minutes")
            .iter()
            .any(|x| x.kind == FactKind::DurationSeconds && x.value == 300.0));
    }

    #[test]
    fn runtime_damage_stays_template_no_invented_value() {
        let f = parse_description("Fires a heavy round that deals <<1>> kinetic damage");
        assert!(f.iter().any(|x| x.kind == FactKind::Template(1)));
        assert!(!f.iter().any(|x| x.kind == FactKind::Magnitude));
    }

    #[test]
    fn flavor_number_yields_nothing() {
        assert!(parse_description("contains 3 lockbox items").is_empty());
    }
}
