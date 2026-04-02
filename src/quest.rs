//! Quest classification and metadata extraction from FQN patterns.
//!
//! All classification is derived from the FQN (Fully Qualified Name) at extraction time.
//! No binary payload parsing needed -- the FQN encodes mission type, faction, planet,
//! class, and companion class.

use regex::Regex;
use std::sync::LazyLock;

/// Structured quest metadata derived from FQN analysis.
pub struct QuestDetails {
    pub fqn: String,
    pub mission_type: String,
    pub faction: Option<String>,
    pub planet: Option<String>,
    pub class_code: Option<String>,
    pub companion_class: Option<String>,
}

static PLANET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"qst\.location\.([^.]+)\.").unwrap(),
        Regex::new(r"qst\.daily_area\.([^.]+)\.").unwrap(),
        Regex::new(r"qst\.exp\.\d+\.([^.]+)\.").unwrap(),
    ]
});

static CLASS_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.class\.([^.]+)\.").unwrap());

static COMPANION_CLASS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"qst\.alliance\.companion\.([^.]+)").unwrap());

const EMPIRE_CLASSES: &[&str] = &[
    "sith_warrior",
    "sith_sorcerer",
    "sith_inquisitor",
    "bounty_hunter",
    "spy",
    "agent",
];

const REPUBLIC_CLASSES: &[&str] = &[
    "jedi_knight",
    "jedi_wizard",
    "jedi_consular",
    "smuggler",
    "trooper",
];

/// Classify a quest FQN into structured metadata.
pub fn classify(fqn: &str, name: &str) -> QuestDetails {
    QuestDetails {
        fqn: fqn.to_string(),
        mission_type: detect_mission_type(fqn, name),
        faction: detect_faction(fqn),
        planet: extract_planet(fqn),
        class_code: extract_class_code(fqn),
        companion_class: extract_companion_class(fqn),
    }
}

/// Detect mission type from FQN pattern and quest name.
///
/// Name-based overrides (bracket prefixes like [HEROIC 2+]) take priority
/// over FQN patterns, since the display name is authoritative.
fn detect_mission_type(fqn: &str, name: &str) -> String {
    let name_lower = name.to_lowercase();

    // Name prefix overrides
    if name_lower.starts_with("[heroic") || name_lower.starts_with("[area") {
        return "heroic".to_string();
    }
    if name_lower.starts_with("[daily") {
        return "daily".to_string();
    }
    if name_lower.starts_with("[weekly") {
        return "weekly".to_string();
    }

    let fqn_lower = fqn.to_lowercase();

    // FQN-based detection (ordered by specificity)
    if fqn_lower.contains(".class.") {
        return "class".to_string();
    }
    if fqn_lower.contains(".world_arc.") {
        return "planetary_arc".to_string();
    }
    if fqn_lower.contains(".world.") && fqn_lower.contains("location") {
        return "planetary".to_string();
    }
    if fqn_lower.starts_with("qst.exp.") {
        return "expansion".to_string();
    }
    if fqn_lower.starts_with("qst.flashpoint.") {
        return "flashpoint".to_string();
    }
    if fqn_lower.starts_with("qst.operation.") {
        return "operation".to_string();
    }
    if fqn_lower.starts_with("qst.event.") {
        return "event".to_string();
    }
    if fqn_lower.starts_with("qst.alliance.") {
        if fqn_lower.contains("companion") {
            return "companion".to_string();
        }
        return "alliance".to_string();
    }
    if fqn_lower.starts_with("qst.ventures.") {
        return "venture".to_string();
    }
    if fqn_lower.starts_with("qst.daily_area.") {
        return "daily".to_string();
    }
    if fqn_lower.starts_with("qst.heroic.") {
        return "heroic".to_string();
    }
    if fqn_lower.starts_with("qst.qtr.") {
        return "weekly".to_string();
    }

    "side".to_string()
}

/// Detect faction from FQN class codes and faction segments.
fn detect_faction(fqn: &str) -> Option<String> {
    let fqn_lower = fqn.to_lowercase();

    if fqn_lower.contains(".imperial")
        || fqn_lower.contains(".empire")
        || fqn_lower.contains("_imp.")
    {
        return Some("empire".to_string());
    }
    if fqn_lower.contains(".republic") || fqn_lower.contains("_rep.") {
        return Some("republic".to_string());
    }

    for c in EMPIRE_CLASSES {
        if fqn_lower.contains(&format!(".{c}.")) {
            return Some("empire".to_string());
        }
    }
    for c in REPUBLIC_CLASSES {
        if fqn_lower.contains(&format!(".{c}.")) {
            return Some("republic".to_string());
        }
    }

    None
}

/// Extract planet name from FQN.
fn extract_planet(fqn: &str) -> Option<String> {
    for re in PLANET_PATTERNS.iter() {
        if let Some(caps) = re.captures(fqn) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}

/// Extract class code from FQN (e.g., "sith_warrior" from ".class.sith_warrior.").
fn extract_class_code(fqn: &str) -> Option<String> {
    CLASS_CODE_RE
        .captures(fqn)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract companion class from alliance companion quest FQN.
fn extract_companion_class(fqn: &str) -> Option<String> {
    COMPANION_CLASS_RE
        .captures(fqn)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_class_quest() {
        let d = classify("qst.class.sith_warrior.act1.the_hunt", "The Hunt");
        assert_eq!(d.mission_type, "class");
        assert_eq!(d.faction.as_deref(), Some("empire"));
        assert_eq!(d.class_code.as_deref(), Some("sith_warrior"));
    }

    #[test]
    fn test_planetary_quest() {
        let d = classify("qst.location.korriban.world.trials", "Trials");
        assert_eq!(d.mission_type, "planetary");
        assert_eq!(d.planet.as_deref(), Some("korriban"));
    }

    #[test]
    fn test_expansion_quest() {
        let d = classify("qst.exp.03.rishi.main_story", "Main Story");
        assert_eq!(d.mission_type, "expansion");
        assert_eq!(d.planet.as_deref(), Some("rishi"));
    }

    #[test]
    fn test_heroic_name_override() {
        let d = classify("qst.location.hoth.world.something", "[HEROIC 2+] Frostbite");
        assert_eq!(d.mission_type, "heroic");
    }

    #[test]
    fn test_companion_class() {
        let d = classify(
            "qst.alliance.companion.bounty_hunter.recruit",
            "To Find a Findsman",
        );
        assert_eq!(d.mission_type, "companion");
        assert_eq!(d.companion_class.as_deref(), Some("bounty_hunter"));
    }

    #[test]
    fn test_daily_area() {
        let d = classify("qst.daily_area.yavin_4.patrol", "Patrol");
        assert_eq!(d.mission_type, "daily");
        assert_eq!(d.planet.as_deref(), Some("yavin_4"));
    }

    #[test]
    fn test_republic_faction() {
        let d = classify("qst.class.jedi_knight.act1.test", "Test");
        assert_eq!(d.faction.as_deref(), Some("republic"));
        assert_eq!(d.class_code.as_deref(), Some("jedi_knight"));
    }

    #[test]
    fn test_neutral_quest() {
        let d = classify("qst.exp.05.ossus.daily", "Daily");
        assert_eq!(d.faction, None);
    }

    #[test]
    fn test_flashpoint() {
        let d = classify("qst.flashpoint.hammer_station.main", "Hammer Station");
        assert_eq!(d.mission_type, "flashpoint");
    }

    #[test]
    fn test_weekly() {
        let d = classify("qst.qtr.conquest.weekly", "[WEEKLY] Conquest");
        assert_eq!(d.mission_type, "weekly");
    }
}
