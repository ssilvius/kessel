//! Decoders for the two GSF requisition-cost singleton prototypes
//! (`scFFComponentsCostPrototype`, `scFFComponentUpgradesCostPrototype`).
//!
//! Both payloads are arrays of records keyed by component content_guid.
//! Components carry one unlock-cost per component; Upgrades carry five
//! tier-cost values per component (standard GSF progression
//! 500 / 1250 / 2500 / 5000 / 7500 requisition).
//!
//! Wire format (verified against firebug-era live extraction 2026-05-24):
//!
//!   `scFFComponentsCostPrototype`:
//!     <CF E0 00 + 8-byte content GUID>
//!     04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01
//!     02 C9 <cost u16 BE>
//!     CB 01 BF 63 42 02 C0 01
//!
//!   `scFFComponentUpgradesCostPrototype`:
//!     <CF E0 00 + 8-byte content GUID>
//!     02 09 05 05                          (5-tier array header)
//!     <tier_idx u8 (01..05)>
//!     04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01
//!     02 C9 <cost u16 BE>
//!     CB 01 BF 63 42 02 C0 01
//!     (... repeat 5 times for each component)
//!
//! Issue: #115 / #172. Singleton bytes pulled from `singletons` table per
//! issue #171's pipeline.

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostKind {
    /// Unlock cost for a GSF component (one per component_guid).
    ComponentUnlock,
    /// Per-tier upgrade cost for a GSF component (five tiers per component).
    TierUpgrade,
}

impl CostKind {
    pub fn as_sql(self) -> &'static str {
        match self {
            CostKind::ComponentUnlock => "component_unlock",
            CostKind::TierUpgrade => "tier_upgrade",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GsfCost {
    /// 16-char hex GUID of the target component / talent.
    pub target_guid: String,
    /// Cost in requisition.
    pub cost: u32,
    /// `ComponentUnlock` for the components payload; `TierUpgrade` for the
    /// upgrades payload. Tier index 1..5 distinguishes tier rows within a
    /// component; ComponentUnlock rows always have `tier = 0`.
    pub tier: u8,
    pub kind: CostKind,
}

/// Format the 8 bytes immediately after a `CF E0 00` marker as the standard
/// 16-char uppercase hex GUID (BE order, matching the convention used by
/// objects.guid).
fn guid_hex(bytes: &[u8]) -> String {
    hex::encode_upper(bytes)
}

/// Decode the components-unlock payload. One cost per `CF E0 00 <guid>`.
pub fn decode_components_cost(payload: &[u8]) -> Result<Vec<GsfCost>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 11 <= payload.len() {
        if payload[i] != 0xCF || payload[i + 1] != 0xE0 || payload[i + 2] != 0x00 {
            i += 1;
            continue;
        }
        let guid_end = i + 11;
        let target_guid = guid_hex(&payload[i + 3..guid_end]);
        // Walk forward up to 30 bytes looking for the first `02 C9 XX YY`
        // cost record. The record is preceded by the CF40 template + a few
        // wrapper bytes (consistent shape per docstring).
        let mut j = guid_end;
        let scan_end = (guid_end + 30).min(payload.len());
        let mut cost: Option<u32> = None;
        while j + 4 <= scan_end {
            if payload[j] == 0x02 && payload[j + 1] == 0xC9 {
                cost = Some(u16::from_be_bytes([payload[j + 2], payload[j + 3]]) as u32);
                break;
            }
            j += 1;
        }
        if let Some(c) = cost {
            out.push(GsfCost {
                target_guid,
                cost: c,
                tier: 0,
                kind: CostKind::ComponentUnlock,
            });
        }
        i = guid_end;
    }
    Ok(out)
}

/// Decode the upgrades payload. Five tier costs per `CF E0 00 <guid>`.
pub fn decode_component_upgrades_cost(payload: &[u8]) -> Result<Vec<GsfCost>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 11 <= payload.len() {
        if payload[i] != 0xCF || payload[i + 1] != 0xE0 || payload[i + 2] != 0x00 {
            i += 1;
            continue;
        }
        let guid_end = i + 11;
        let target_guid = guid_hex(&payload[i + 3..guid_end]);
        // Walk forward through the 5-tier array, collecting `02 C9 XX YY`
        // cost records in order. Stop at the next `CF E0` marker or after 5
        // costs collected.
        let mut j = guid_end;
        let mut tier: u8 = 0;
        while j + 4 <= payload.len() && tier < 5 {
            // Stop at next component marker
            if j + 3 <= payload.len()
                && payload[j] == 0xCF
                && payload[j + 1] == 0xE0
                && payload[j + 2] == 0x00
            {
                break;
            }
            if payload[j] == 0x02 && payload[j + 1] == 0xC9 {
                tier += 1;
                let cost = u16::from_be_bytes([payload[j + 2], payload[j + 3]]) as u32;
                out.push(GsfCost {
                    target_guid: target_guid.clone(),
                    cost,
                    tier,
                    kind: CostKind::TierUpgrade,
                });
                j += 4;
                continue;
            }
            j += 1;
        }
        i = j;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real first-component slice of scFFComponentsCostPrototype (verified
    /// against live spice extract 2026-05-24). One component, cost 1000.
    fn components_first_record() -> Vec<u8> {
        // CF E0 00 <8-byte GUID> + record body + cost (02 C9 03 E8 = 1000 BE)
        // + trailer
        let hex = "CF E0 00 00 FA F1 6B B9 AF 04 03 \
                   CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 03 E8 \
                   CB 01 BF 63 42 02 C0 01";
        hex_to_bytes(hex)
    }

    /// Real first-component slice of scFFComponentUpgradesCostPrototype.
    /// One component with 5 tier costs (500/1250/2500/5000/7500).
    fn upgrades_first_record() -> Vec<u8> {
        let hex = "CF E0 00 00 FA F1 6B B9 AF 02 09 05 05 01 \
                   04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 01 F4 \
                   CB 01 BF 63 42 02 C0 01 02 \
                   04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 04 E2 \
                   CB 01 BF 63 42 02 C0 01 03 \
                   04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 09 C4 \
                   CB 01 BF 63 42 02 C0 01 04 \
                   04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 13 88 \
                   CB 01 BF 63 42 02 C0 01 05 \
                   04 03 CF 40 00 00 41 95 3B 9C 71 02 C0 01 01 \
                   02 C9 1D 4C \
                   CB 01 BF 63 42 02 C0 01";
        hex_to_bytes(hex)
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|s| u8::from_str_radix(s, 16).expect("hex"))
            .collect()
    }

    #[test]
    fn decode_components_cost_first_record_is_1000_req() {
        let bytes = components_first_record();
        let costs = decode_components_cost(&bytes).unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].target_guid, "00FAF16BB9AF0403");
        assert_eq!(costs[0].cost, 1000);
        assert_eq!(costs[0].tier, 0);
        assert_eq!(costs[0].kind, CostKind::ComponentUnlock);
    }

    #[test]
    fn decode_component_upgrades_cost_first_record_is_5_tiers() {
        let bytes = upgrades_first_record();
        let costs = decode_component_upgrades_cost(&bytes).unwrap();
        assert_eq!(costs.len(), 5);
        let expected_costs = [500u32, 1250, 2500, 5000, 7500];
        for (i, expected) in expected_costs.iter().enumerate() {
            assert_eq!(costs[i].tier, (i + 1) as u8);
            assert_eq!(costs[i].cost, *expected, "tier {} cost", i + 1);
            assert_eq!(costs[i].kind, CostKind::TierUpgrade);
        }
        // All 5 tiers share the same target_guid.
        let g0 = &costs[0].target_guid;
        assert!(costs.iter().all(|c| &c.target_guid == g0));
    }

    #[test]
    fn decode_handles_empty_payload() {
        assert!(decode_components_cost(&[]).unwrap().is_empty());
        assert!(decode_component_upgrades_cost(&[]).unwrap().is_empty());
    }

    #[test]
    fn cost_kind_sql_round_trips() {
        assert_eq!(CostKind::ComponentUnlock.as_sql(), "component_unlock");
        assert_eq!(CostKind::TierUpgrade.as_sql(), "tier_upgrade");
    }
}
