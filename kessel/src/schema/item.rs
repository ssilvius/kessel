//! Item classification from FQN patterns.
//!
//! Most item metadata (slot, rating, rarity, source, armor weight, crew skill)
//! is derivable from the FQN. Only set name and set bonus require GOM payload
//! parsing -- those land in a follow-up.

use regex::Regex;
use std::sync::LazyLock;

pub struct ItemDetails {
    pub fqn: String,
    /// Top-level item category from FQN segment 2: gear | mod | schematic |
    /// decoration | consumable | material | mtx | npc | loot | reputation |
    /// companion | custom | quest_token | test | other.
    pub item_kind: String,
    /// Equipment slot for wearable gear: chest, head, legs, hands, feet, waist,
    /// wrists, ear, implant, relic, mainhand, offhand, shield.
    pub slot: Option<String>,
    /// Weapon subtype: lightsaber, polesaber, blaster, cannon, vibroknife,
    /// rifle, shotgun, sniper, electrostaff, techblade, techstaff.
    pub weapon_type: Option<String>,
    /// Armor weight class for chest/head/legs/hands/feet armor.
    pub armor_weight: Option<String>,
    /// Rarity tier: premium, prototype, artifact, legendary.
    pub rarity: Option<String>,
    /// Item level extracted from `ilvl_NNNN` or `level_NNN` segments.
    pub item_level: Option<u32>,
    /// Source bucket: flashpoint, operation, conquest, pvp, raid, heroic,
    /// command, mtx, mission, vendor, quest, world, schematic.
    pub source: Option<String>,
    /// True for itm.schem.* schematic objects.
    pub is_schematic: bool,
    /// Crew skill associated with the item (creator skill for schematics,
    /// gathered/produced skill for materials and crafted gear).
    pub crew_skill: Option<String>,
}

const SLOT_TOKENS: &[&str] = &[
    "chest", "head", "legs", "hands", "feet", "waist", "wrists", "wrist", "ear", "earpiece",
    "implant", "relic", "mainhand", "offhand", "shield",
];

const WEAPON_TOKENS: &[&str] = &[
    "lightsaber",
    "polesaber",
    "doublesaber",
    "blaster",
    "cannon",
    "vibroknife",
    "vibrosword",
    "rifle",
    "shotgun",
    "sniper",
    "electrostaff",
    "techblade",
    "techstaff",
    "bowcaster",
];

const ARMOR_WEIGHTS: &[&str] = &["light", "medium", "heavy"];

const RARITY_TOKENS: &[&str] = &["premium", "prototype", "artifact", "legendary"];

const CREW_SKILLS: &[&str] = &[
    "armormech",
    "armstech",
    "artifice",
    "biochem",
    "cybertech",
    "synthweaving",
];

static ILVL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:ilvl_|level_)(\d{2,4})").unwrap());

pub fn classify(fqn: &str) -> ItemDetails {
    let segments: Vec<&str> = fqn.split('.').collect();
    let lower = fqn.to_lowercase();
    let is_schematic = segments.get(1).copied() == Some("schem");

    ItemDetails {
        fqn: fqn.to_string(),
        item_kind: detect_item_kind(&segments, is_schematic),
        slot: detect_slot(&lower),
        weapon_type: detect_weapon_type(&lower),
        armor_weight: detect_armor_weight(&lower),
        rarity: detect_rarity(&lower),
        item_level: extract_item_level(&lower),
        source: detect_source(&segments, &lower),
        is_schematic,
        crew_skill: detect_crew_skill(&segments, &lower),
    }
}

fn detect_item_kind(segments: &[&str], is_schematic: bool) -> String {
    if is_schematic {
        return "schematic".to_string();
    }
    match segments.get(1).copied().unwrap_or("") {
        "gen" | "endgame" | "eq" => "gear".to_string(),
        "mod" => detect_mod_subkind(segments),
        "tactical" => "tactical".to_string(),
        "setbonus" => "setbonus".to_string(),
        "stronghold" => "decoration".to_string(),
        "potion" => "consumable".to_string(),
        "mat" => "material".to_string(),
        "mtx" => "mtx".to_string(),
        "npc" => "npc".to_string(),
        "loot" => "loot".to_string(),
        "reputation" => "reputation".to_string(),
        "companion" => "companion".to_string(),
        "custom" => "custom".to_string(),
        "has_item" => "quest_token".to_string(),
        "test" => "test".to_string(),
        _ => "other".to_string(),
    }
}

/// Split `itm.mod.*` into the modification subtypes a player-facing search
/// would care about: enhancement, augment, augment_kit. Everything else under
/// itm.mod.* (armoring, mod, hilt, barrel, modulator, support, harness,
/// overlay, ...) keeps the umbrella "mod" kind -- they're all gear modifications
/// installed in different slots, but the UI groups them as a single facet.
fn detect_mod_subkind(segments: &[&str]) -> String {
    match segments.get(2).copied().unwrap_or("") {
        "enhancement" => "enhancement".to_string(),
        "augment" => "augment".to_string(),
        "augment_kit" => "augment_kit".to_string(),
        _ => "mod".to_string(),
    }
}

fn detect_slot(fqn_lower: &str) -> Option<String> {
    for token in SLOT_TOKENS {
        if contains_segment(fqn_lower, token) || fqn_lower.contains(&format!("_{token}")) {
            let canon = match *token {
                "wrist" => "wrists",
                "earpiece" => "ear",
                t => t,
            };
            return Some(canon.to_string());
        }
    }
    if fqn_lower.contains("trinket_earpiece") {
        return Some("ear".to_string());
    }
    None
}

fn detect_weapon_type(fqn_lower: &str) -> Option<String> {
    for token in WEAPON_TOKENS {
        if contains_segment(fqn_lower, token) || fqn_lower.contains(&format!("_{token}")) {
            return Some((*token).to_string());
        }
    }
    None
}

fn detect_armor_weight(fqn_lower: &str) -> Option<String> {
    for w in ARMOR_WEIGHTS {
        if contains_segment(fqn_lower, w)
            || fqn_lower.contains(&format!("_{w}_"))
            || fqn_lower.contains(&format!(".{w}_"))
        {
            return Some((*w).to_string());
        }
    }
    None
}

fn detect_rarity(fqn_lower: &str) -> Option<String> {
    for r in RARITY_TOKENS {
        if contains_segment(fqn_lower, r) {
            return Some((*r).to_string());
        }
    }
    None
}

fn extract_item_level(fqn_lower: &str) -> Option<u32> {
    ILVL_RE
        .captures(fqn_lower)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Decode item_level from an itm.* GOM payload as a fallback when the FQN
/// has no `ilvl_NNNN` segment. The marker is a 4-byte sentinel `06 43 02 02`
/// followed by the ilvl as a single u8 (verified empirically against ~40K
/// gear items with FQN-derived ilvl, then confirmed to recover ~53% of the
/// FQN-NULL gear pool). Validates the byte to a sane SWTOR ilvl range
/// (1..=250) so a chance pattern in a non-gear payload doesn't poison the
/// column.
pub fn extract_item_level_from_payload(payload: &[u8]) -> Option<u32> {
    const MARKER: [u8; 4] = [0x06, 0x43, 0x02, 0x02];
    let mut i = 0;
    while i + MARKER.len() < payload.len() {
        if payload[i..i + MARKER.len()] == MARKER {
            let ilvl = payload[i + MARKER.len()];
            if (1..=250).contains(&ilvl) {
                return Some(ilvl as u32);
            }
        }
        i += 1;
    }
    None
}

fn detect_source(segments: &[&str], fqn_lower: &str) -> Option<String> {
    let seg2 = segments.get(2).copied().unwrap_or("");
    let s = match seg2 {
        "lots" => Some("operation_or_flashpoint"),
        "raid" => Some("raid"),
        "heroic" => Some("heroic"),
        "command" => Some("command"),
        "pvp" | "pvp_imp" | "pvp_rep" => Some("pvp"),
        "flashpoint" => Some("flashpoint"),
        "quest" | "quest_shared" | "quest_wpn" | "quest_imp" => Some("quest"),
        "bis_shared" | "bis_wpn" => Some("bis"),
        "random" | "random_shared" => Some("random"),
        "sow" => Some("sow"),
        _ => None,
    };
    if s.is_some() {
        return s.map(String::from);
    }
    if fqn_lower.contains(".flashpoint.") {
        return Some("flashpoint".to_string());
    }
    if fqn_lower.contains(".operation.") {
        return Some("operation".to_string());
    }
    if fqn_lower.contains(".conquest.") {
        return Some("conquest".to_string());
    }
    if fqn_lower.starts_with("itm.mtx.") {
        return Some("mtx".to_string());
    }
    None
}

fn detect_crew_skill(segments: &[&str], fqn_lower: &str) -> Option<String> {
    for skill in CREW_SKILLS {
        if segments.iter().any(|s| s.eq_ignore_ascii_case(skill))
            || fqn_lower.contains(&format!(".{skill}."))
        {
            return Some((*skill).to_string());
        }
    }
    None
}

fn contains_segment(fqn_lower: &str, token: &str) -> bool {
    fqn_lower
        .split('.')
        .any(|seg| seg == token || seg.split('_').any(|p| p == token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_endgame_lightsaber() {
        let d = classify("itm.endgame.cdps1.lightsaber.artifact.01x1i");
        assert_eq!(d.item_kind, "gear");
        assert_eq!(d.weapon_type.as_deref(), Some("lightsaber"));
        assert_eq!(d.rarity.as_deref(), Some("artifact"));
    }

    #[test]
    fn classifies_chest_armor_with_ilvl() {
        let d =
            classify("itm.gen.lots.armor.war_kni_tank.flashpoint.ilvl_0165.premium.armor_chest");
        assert_eq!(d.slot.as_deref(), Some("chest"));
        assert_eq!(d.rarity.as_deref(), Some("premium"));
        assert_eq!(d.item_level, Some(165));
        assert_eq!(d.source.as_deref(), Some("operation_or_flashpoint"));
    }

    #[test]
    fn classifies_schematic() {
        let d = classify("itm.schem.gen.quest_imp.rdps1.chest.heavy.premium.03x1_craft");
        assert!(d.is_schematic);
        assert_eq!(d.item_kind, "schematic");
        assert_eq!(d.slot.as_deref(), Some("chest"));
        assert_eq!(d.armor_weight.as_deref(), Some("heavy"));
    }

    #[test]
    fn classifies_synthweaving_with_skill() {
        let d = classify("itm.gen.synthweaving.hybrid_tank_will.ilvl_081.premium.light_chest");
        assert_eq!(d.crew_skill.as_deref(), Some("synthweaving"));
        assert_eq!(d.armor_weight.as_deref(), Some("light"));
        assert_eq!(d.slot.as_deref(), Some("chest"));
    }

    #[test]
    fn classifies_mtx_armor() {
        let d = classify("itm.mtx.armor.storefront.enigmatic_hero.hands");
        assert_eq!(d.item_kind, "mtx");
        assert_eq!(d.slot.as_deref(), Some("hands"));
        assert_eq!(d.source.as_deref(), Some("mtx"));
    }

    #[test]
    fn classifies_relic() {
        let d = classify("itm.gen.quest.relics.ilvl_0028.prototype.relic_defense_proc");
        assert_eq!(d.slot.as_deref(), Some("relic"));
        assert_eq!(d.rarity.as_deref(), Some("prototype"));
        assert_eq!(d.item_level, Some(28));
    }

    #[test]
    fn classifies_decoration() {
        let d = classify("itm.stronghold.environmental.plants.manaan.spiraled_seaweed_1");
        assert_eq!(d.item_kind, "decoration");
        assert_eq!(d.slot, None);
    }

    #[test]
    fn classifies_mod_no_slot() {
        let d = classify("itm.mod.color_crystal.att_pwr.green.artifact.basemod_03");
        assert_eq!(d.item_kind, "mod");
        assert_eq!(d.rarity.as_deref(), Some("artifact"));
    }

    #[test]
    fn extracts_item_level_from_marker() {
        // 06 43 02 02 <ilvl> at offset 5; ilvl=132.
        let payload = vec![
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x06, 0x43, 0x02, 0x02, 132, 0x00,
        ];
        assert_eq!(extract_item_level_from_payload(&payload), Some(132));
    }

    #[test]
    fn returns_none_when_marker_absent() {
        let payload = b"no_marker_here_just_random_bytes_present_at_all".to_vec();
        assert_eq!(extract_item_level_from_payload(&payload), None);
    }

    #[test]
    fn returns_none_for_truncated_payload() {
        // Marker prefix but cut off before the ilvl byte.
        assert_eq!(
            extract_item_level_from_payload(&[0x06, 0x43, 0x02, 0x02]),
            None
        );
        assert_eq!(extract_item_level_from_payload(&[]), None);
    }

    #[test]
    fn rejects_out_of_range_ilvl() {
        // ilvl=0 is below valid range -- skip and keep scanning.
        let payload = vec![
            0x06, 0x43, 0x02, 0x02, 0, 0xFF, 0x06, 0x43, 0x02, 0x02, 75, 0x00,
        ];
        assert_eq!(extract_item_level_from_payload(&payload), Some(75));
    }

    #[test]
    fn returns_first_match_when_multiple_markers_present() {
        let mut payload = vec![0x00; 50];
        payload[10..15].copy_from_slice(&[0x06, 0x43, 0x02, 0x02, 100]);
        payload[30..35].copy_from_slice(&[0x06, 0x43, 0x02, 0x02, 200]);
        assert_eq!(extract_item_level_from_payload(&payload), Some(100));
    }

    #[test]
    fn classifies_tactical() {
        let d = classify("itm.tactical.sow.juggernaut.grit_teeth");
        assert_eq!(d.item_kind, "tactical");
    }

    #[test]
    fn classifies_setbonus() {
        let d = classify("itm.setbonus.sample_set.ear");
        assert_eq!(d.item_kind, "setbonus");
    }

    #[test]
    fn classifies_mod_enhancement_subkind() {
        let d = classify("itm.mod.enhancement.sample_tier1.sample_name");
        assert_eq!(d.item_kind, "enhancement");
    }

    #[test]
    fn classifies_mod_augment_subkind() {
        let d = classify("itm.mod.augment.sample_grade.sample_name");
        assert_eq!(d.item_kind, "augment");
    }

    #[test]
    fn classifies_mod_augment_kit_subkind() {
        let d = classify("itm.mod.augment_kit.sample_grade");
        assert_eq!(d.item_kind, "augment_kit");
    }

    #[test]
    fn mod_other_subkinds_stay_umbrella_mod() {
        // armoring/hilt/barrel/modulator/support/harness/overlay etc.
        // all roll up as "mod" -- player-facing search treats them as one facet.
        for fqn in [
            "itm.mod.armoring.sample.tier1",
            "itm.mod.hilt.sample.tier1",
            "itm.mod.barrel.sample.tier1",
            "itm.mod.modulator.sample.tier1",
        ] {
            assert_eq!(classify(fqn).item_kind, "mod", "fqn={fqn}");
        }
    }

    #[test]
    fn classifies_offhand_with_weapon_type() {
        let d = classify("itm.gen.bis_wpn.tdps1a.blaster_offhand.prototype.07x1i");
        assert_eq!(d.slot.as_deref(), Some("offhand"));
        assert_eq!(d.weapon_type.as_deref(), Some("blaster"));
    }
}
