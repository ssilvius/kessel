//! Gift item classification — shared structure between kessel (Rust) and frontend (TS).
//!
//! FQN pattern: `itm.companion.gift.{type}.{quality}_rank{rank}_v1`
//! Both sides parse the same FQN fragments into type/quality/rank → game_id.

#![allow(dead_code)] // WIP: not yet integrated into main extraction pipeline

use std::collections::BTreeMap;
use std::fmt;

use anyhow::Result;
use serde::Serialize;

/// Gift types — matches `GiftType` in `data/gift-calculator.ts`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GiftType {
    Weapon,
    Technology,
    Luxury,
    Courting,
    CulturalArtifact,
    ImperialMemorabilia,
    RepublicMemorabilia,
    MilitaryGear,
    UnderworldGood,
    Trophy,
    Delicacies,
    Maintenance,
}

impl GiftType {
    /// Parse from FQN segment (e.g. "weapon", "cultural_artifact")
    fn from_fqn(s: &str) -> Option<Self> {
        match s {
            "weapon" => Some(Self::Weapon),
            "technology" => Some(Self::Technology),
            "luxury" => Some(Self::Luxury),
            "courting" => Some(Self::Courting),
            "cultural_artifact" => Some(Self::CulturalArtifact),
            "imperial_memorabilia" => Some(Self::ImperialMemorabilia),
            "republic_memorabilia" => Some(Self::RepublicMemorabilia),
            "military_gear" => Some(Self::MilitaryGear),
            "underworld_good" => Some(Self::UnderworldGood),
            "trophy" => Some(Self::Trophy),
            "delicacies" => Some(Self::Delicacies),
            "maintenance" => Some(Self::Maintenance),
            _ => None,
        }
    }
}

impl fmt::Display for GiftType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Weapon => write!(f, "weapon"),
            Self::Technology => write!(f, "technology"),
            Self::Luxury => write!(f, "luxury"),
            Self::Courting => write!(f, "courting"),
            Self::CulturalArtifact => write!(f, "cultural_artifact"),
            Self::ImperialMemorabilia => write!(f, "imperial_memorabilia"),
            Self::RepublicMemorabilia => write!(f, "republic_memorabilia"),
            Self::MilitaryGear => write!(f, "military_gear"),
            Self::UnderworldGood => write!(f, "underworld_good"),
            Self::Trophy => write!(f, "trophy"),
            Self::Delicacies => write!(f, "delicacies"),
            Self::Maintenance => write!(f, "maintenance"),
        }
    }
}

/// Gift qualities — matches `GiftQuality` in `data/gift-calculator.ts`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GiftQuality {
    Premium,
    Prototype,
    Artifact,
    Legendary,
}

impl GiftQuality {
    /// Parse from FQN segment (e.g. "premium", "artifact")
    fn from_fqn(s: &str) -> Option<Self> {
        match s {
            "premium" => Some(Self::Premium),
            "prototype" => Some(Self::Prototype),
            "artifact" => Some(Self::Artifact),
            "legendary" => Some(Self::Legendary),
            _ => None,
        }
    }
}

impl fmt::Display for GiftQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Premium => write!(f, "premium"),
            Self::Prototype => write!(f, "prototype"),
            Self::Artifact => write!(f, "artifact"),
            Self::Legendary => write!(f, "legendary"),
        }
    }
}

/// Parsed gift item from FQN fragments
#[derive(Debug, Clone)]
pub struct GiftItem {
    pub gift_type: GiftType,
    pub quality: GiftQuality,
    pub rank: u8,
    pub game_id: String,
    pub fqn: String,
}

/// Parse a gift item FQN into its fragments.
/// Pattern: `itm.companion.gift.{type}.{quality}_rank{rank}_{suffix}`
pub fn parse_gift_fqn(fqn: &str) -> Option<(GiftType, GiftQuality, u8)> {
    let stripped = fqn.strip_prefix("itm.companion.gift.")?;

    // Split: "weapon.premium_rank1_v1" → type="weapon", rest="premium_rank1_v1"
    let dot = stripped.find('.')?;
    let type_str = &stripped[..dot];
    let rest = &stripped[dot + 1..];

    let gift_type = GiftType::from_fqn(type_str)?;

    // Parse: "premium_rank1_v1" → quality="premium", rank=1
    let rank_pos = rest.find("_rank")?;
    let quality_str = &rest[..rank_pos];
    let after_rank = &rest[rank_pos + 5..]; // skip "_rank"

    let quality = GiftQuality::from_fqn(quality_str)?;

    // Rank is the digit(s) before the next underscore
    let rank_end = after_rank.find('_').unwrap_or(after_rank.len());
    let rank: u8 = after_rank[..rank_end].parse().ok()?;

    Some((gift_type, quality, rank))
}

/// Nested map: type → quality → rank → game_id
/// Matches the TS structure `GIFT_GAME_IDS` in `data/gift-icons.ts`
pub type GiftGameIdMap = BTreeMap<GiftType, BTreeMap<GiftQuality, BTreeMap<u8, String>>>;

/// Build the gift game_id map from spice.sqlite.
/// Queries all `itm.companion.gift.*` objects and groups by type/quality/rank.
/// Prefers `_v1` suffix over `_vendor`/`_bol`.
pub fn build_gift_map(db_path: &std::path::Path) -> Result<GiftGameIdMap> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let mut stmt =
        conn.prepare("SELECT fqn, game_id FROM objects WHERE fqn LIKE 'itm.companion.gift.%'")?;

    let mut map = GiftGameIdMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (fqn, game_id) = row?;

        if let Some((gift_type, quality, rank)) = parse_gift_fqn(&fqn) {
            let entry = map
                .entry(gift_type)
                .or_default()
                .entry(quality)
                .or_default()
                .entry(rank)
                .or_insert_with(|| game_id.clone());

            // Prefer _v1 over other suffixes
            if fqn.ends_with("_v1") {
                *entry = game_id;
            }
        }
    }

    Ok(map)
}

/// Serialize the gift map to JSON matching the TS `GIFT_GAME_IDS` structure.
/// Keys use snake_case strings (matching TS type literals).
pub fn gift_map_to_json(map: &GiftGameIdMap) -> Result<String> {
    // Convert BTreeMap keys to string keys for JSON output
    let mut outer: BTreeMap<String, BTreeMap<String, BTreeMap<String, &str>>> = BTreeMap::new();

    for (gift_type, qualities) in map {
        let mut quality_map: BTreeMap<String, BTreeMap<String, &str>> = BTreeMap::new();
        for (quality, ranks) in qualities {
            let mut rank_map: BTreeMap<String, &str> = BTreeMap::new();
            for (rank, game_id) in ranks {
                rank_map.insert(rank.to_string(), game_id.as_str());
            }
            quality_map.insert(quality.to_string(), rank_map);
        }
        outer.insert(gift_type.to_string(), quality_map);
    }

    Ok(serde_json::to_string_pretty(&outer)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_gift() {
        let result = parse_gift_fqn("itm.companion.gift.weapon.premium_rank1_v1");
        assert_eq!(result, Some((GiftType::Weapon, GiftQuality::Premium, 1)));
    }

    #[test]
    fn parse_vendor_suffix() {
        let result = parse_gift_fqn("itm.companion.gift.luxury.artifact_rank3_vendor");
        assert_eq!(result, Some((GiftType::Luxury, GiftQuality::Artifact, 3)));
    }

    #[test]
    fn parse_rank6() {
        let result = parse_gift_fqn("itm.companion.gift.delicacies.legendary_rank6_v1");
        assert_eq!(
            result,
            Some((GiftType::Delicacies, GiftQuality::Legendary, 6))
        );
    }

    #[test]
    fn parse_compound_type() {
        let result = parse_gift_fqn("itm.companion.gift.cultural_artifact.prototype_rank3_v1");
        assert_eq!(
            result,
            Some((GiftType::CulturalArtifact, GiftQuality::Prototype, 3))
        );
    }

    #[test]
    fn reject_non_gift() {
        assert_eq!(parse_gift_fqn("itm.gen.lots.generic"), None);
    }

    #[test]
    fn reject_unknown_quality() {
        // "standard" quality exists in game but not in our type system
        assert_eq!(
            parse_gift_fqn("itm.companion.gift.weapon.standard_rank1_v1"),
            None
        );
    }
}
