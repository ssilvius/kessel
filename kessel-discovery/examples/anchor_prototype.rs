//! Prototype of the description-anchoring parser (planning artifact for the
//! description-anchoring epic). Parses literal numbers + `<<N>>` template
//! ordinals out of a localized description string into typed numeric facts.
//!
//! This is the proof that the displayed VALUES (durations, percentages, ranges,
//! counts, named-stat magnitudes) are recoverable directly from the strings --
//! NOT runtime/external. Method confirmed corpus-wide: 93% of abl+tal id1=1
//! descriptions yield >=1 fact (reflection 019ee785).
//!
//! Runs in-process oracle assertions then a corpus precision pass.
//! cargo run -p kessel-discovery --example anchor_prototype

use anyhow::Result;
use regex::Regex;
use rusqlite::{Connection, OpenFlags};

const DB: &str = "/Users/seansilvius/swtor/data/spice-7.9.a-v7.sqlite";

#[derive(Debug, Clone, PartialEq)]
enum FactKind {
    Percent,
    DurationSeconds,
    RangeMeters,
    Count,
    Magnitude,
    /// A `<<N>>` template token -- value filled at runtime (level-scaled
    /// damage/heal) or recoverable via the payload_ordinal substrate (duration).
    Template(u32),
}

#[derive(Debug, Clone, PartialEq)]
struct Fact {
    kind: FactKind,
    /// Literal numeric value (normalized to base unit: seconds, meters, points).
    /// Zero for `Template` facts.
    value: f64,
    /// The stat/unit word attached to the number (`power`, `seconds`, `chance`).
    label: String,
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

struct Parser {
    num_unit: Regex,
    stat_phrase: Regex,
}

impl Parser {
    fn new() -> Result<Self> {
        Ok(Self {
            // <<N>> token | N% | N word
            num_unit: Regex::new(
                r"(?:<<(\d+)(?:\[[^\]]*\])?>>|(\d+(?:\.\d+)?)\s*%|(\d+(?:\.\d+)?))\s*([A-Za-z]+)?",
            )?,
            // "increases X (Rating) by N(%)" / "grant N ... <stat>" handled by num_unit; this
            // catches the stat-BEFORE-number orientation.
            stat_phrase: Regex::new(
                r"(?i)(?:increase[sd]?|reduce[sd]?|by)\s+(?:your\s+)?([A-Z][A-Za-z ]{2,24}?)\s+(?:Rating\s+)?by\s+(\d+(?:\.\d+)?)\s*(%?)",
            )?,
        })
    }

    fn parse(&self, text: &str) -> Vec<Fact> {
        let mut facts = Vec::new();
        for c in self.stat_phrase.captures_iter(text) {
            let stat = c[1].trim().to_lowercase();
            let v: f64 = c[2].parse().unwrap_or(0.0);
            let pct = !c[3].is_empty();
            facts.push(Fact {
                kind: if pct {
                    FactKind::Percent
                } else {
                    FactKind::Magnitude
                },
                value: v,
                label: stat,
            });
        }
        for c in self.num_unit.captures_iter(text) {
            if let Some(ord) = c.get(1) {
                let n: u32 = ord.as_str().parse().unwrap_or(0);
                facts.push(Fact {
                    kind: FactKind::Template(n),
                    value: 0.0,
                    label: c
                        .get(4)
                        .map(|m| m.as_str().to_lowercase())
                        .unwrap_or_default(),
                });
            } else if let Some(p) = c.get(2) {
                facts.push(Fact {
                    kind: FactKind::Percent,
                    value: p.as_str().parse().unwrap_or(0.0),
                    label: c
                        .get(4)
                        .map(|m| m.as_str().to_lowercase())
                        .unwrap_or_else(|| "pct".into()),
                });
            } else if let Some(lit) = c.get(3) {
                let v: f64 = lit.as_str().parse().unwrap_or(0.0);
                let w = c
                    .get(4)
                    .map(|m| m.as_str().to_lowercase())
                    .unwrap_or_default();
                let kind = if let Some(mult) = duration_mult(&w) {
                    Some((FactKind::DurationSeconds, v * mult))
                } else if is_range(&w) {
                    Some((FactKind::RangeMeters, v))
                } else if is_count(&w) {
                    Some((FactKind::Count, v))
                } else if is_stat(&w) {
                    Some((FactKind::Magnitude, v))
                } else {
                    None // bare number with non-unit word -> flavor/noise, skip
                };
                if let Some((k, val)) = kind {
                    facts.push(Fact {
                        kind: k,
                        value: val,
                        label: w,
                    });
                }
            }
        }
        facts
    }
}

fn main() -> Result<()> {
    let p = Parser::new()?;

    // ---- oracle assertions (the spec) ----
    let f =
        p.parse("Increases your chance to dodge by 30% for <<1>> seconds. Once every 20 seconds.");
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::Percent && x.value == 30.0));
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::DurationSeconds && x.value == 20.0));
    assert!(f.iter().any(|x| matches!(x.kind, FactKind::Template(1))));

    let f = p.parse("stealing health from up to 4 targets within an 8 meter radius");
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::Count && x.value == 4.0));
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::RangeMeters && x.value == 8.0));

    let f = p.parse("grant 510 Power for 6 seconds");
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::Magnitude && x.value == 510.0 && x.label == "power"));
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::DurationSeconds && x.value == 6.0));

    // stat BEFORE the number
    let f = p.parse("Increases Critical Rating by 40 for <<1>> seconds.");
    assert!(f
        .iter()
        .any(|x| x.kind == FactKind::Magnitude && x.value == 40.0 && x.label.contains("critical")));

    // runtime damage stays a template, no invented value
    let f = p.parse("Fires a heavy round that deals <<1>> kinetic damage.");
    assert!(f.iter().any(|x| matches!(x.kind, FactKind::Template(1))));
    assert!(!f.iter().any(|x| x.kind == FactKind::Magnitude));
    println!("ORACLE ASSERTIONS PASSED");

    // ---- corpus precision pass ----
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(text_raw, text) FROM strings WHERE locale='en-us' AND id1=1 \
         AND COALESCE(text_raw, text) GLOB '*[0-9]*' \
         AND (fqn GLOB 'str.abl.*' OR fqn GLOB 'str.tal.*' OR fqn GLOB 'str.itm.*')",
    )?;
    let texts: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    let (mut with, mut facts) = (0u64, 0u64);
    for t in &texts {
        let fs = p.parse(t);
        if !fs.is_empty() {
            with += 1;
        }
        facts += fs.len() as u64;
    }
    println!(
        "corpus: {} strings, {} yielded >=1 fact ({:.0}%), {} facts total",
        texts.len(),
        with,
        100.0 * with as f64 / texts.len() as f64,
        facts
    );
    Ok(())
}
