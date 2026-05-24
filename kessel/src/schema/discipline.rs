//! Decoder for `dis.*` PBUK payloads (discipline records).
//!
//! Each `dis.<class>.<discipline>` object contains:
//!   - Short codename (`power_pyrotech`, `jugg_immortal`, `sorc_lightning`)
//!   - 2 apc.* references: discipline icon + mod-tree visual
//!   - 24 ability/talent GUIDs (the discipline's mod pool)
//!   - 8 tier triplets (3 choices per tier + per-tier level gate, with UI ordering)
//!   - 24 (index, level) pairs (redundant with the tier triplets but explicit)
//!   - 8 default-choice pairs (which mod is the auto-selected default per tier)
//!   - 1 signature ability GUID (the iconic ability for the discipline)
//!
//! Format and validation reference: `docs/probes/dis-payload-format.md`.
//! All 48 sampled disciplines share the same shape (51 CF E0 markers total,
//! 8 tier triplets, fixed transition marker, exactly 1 trailing signature ref).
//!
//! The 2 unresolved CF E0 refs per discipline (Heat Chain category) are
//! emitted with their raw GUIDs and surface as the broken edges issue #179
//! investigates.

use anyhow::{bail, Result};

/// One decoded discipline payload.
#[derive(Debug, Clone)]
pub struct DisciplineRecord {
    /// Short codename from the length-prefixed string after the class marker.
    pub codename: String,
    /// Icon apc.* GUID (16 hex chars, BE order, `E000` prefix + 6-byte tail).
    pub icon_apc_guid: String,
    /// Mod-tree visual apc.* GUID.
    pub mod_tree_apc_guid: String,
    /// 24 entries in declaration order. `mods[i].index == i + 1`.
    pub mods: Vec<DisciplineModEntry>,
    /// 8 tier triplets in tier-ordinal order.
    pub tiers: Vec<DisciplineTier>,
    /// 8 (level, index) default-choice pairs. The mod index that is the
    /// default selection at the given tier level. Section J of the format;
    /// the "default" interpretation is plausible from structure but not
    /// validated against in-game behavior (see probe doc open questions).
    pub defaults: Vec<DisciplineDefault>,
    /// Signature ability GUID (the discipline-defining auto-granted ability).
    pub signature_ability_guid: String,
}

#[derive(Debug, Clone)]
pub struct DisciplineModEntry {
    /// 1-based index used by tier/default refs.
    pub index: u8,
    /// Full 16-char hex GUID (`E000` + 6-byte tail BE).
    pub guid: String,
    /// Player level required to unlock (from the index-to-level map).
    pub level: u8,
}

#[derive(Debug, Clone)]
pub struct DisciplineTier {
    /// 1-based tier ordinal.
    pub ordinal: u8,
    /// Player level the tier unlocks at. Redundant with the per-mod level in
    /// `DisciplineModEntry::level` for any mod in this tier, but useful as a
    /// typed tier-level accessor for consumers of the decoder API.
    #[allow(dead_code)]
    pub level: u8,
    /// 3 mod indices (1-based) for left / middle / right UI choices.
    /// Order follows the in-game UI layout, not numerical index order.
    pub choice_indices: [u8; 3],
}

#[derive(Debug, Clone)]
pub struct DisciplineDefault {
    /// Player level the default-choice rule applies at. Redundant with the
    /// per-mod level via `default_mod_index`, but exposed for typed access.
    #[allow(dead_code)]
    pub level: u8,
    pub default_mod_index: u8,
}

const CLASS_MARKER: &[u8] = &[0xCF, 0x40, 0x00, 0x00, 0x41, 0xFC, 0x3C, 0x7A, 0x20];
/// 6-byte array opener that immediately precedes the first tier triplet.
/// The bytes between the transition marker and this opener vary by discipline
/// (sometimes 1 byte, sometimes 3), so anchor on the opener itself.
const TIER_ARRAY_OPENER: &[u8] = &[0x08, 0x02, 0x07, 0x08, 0x08, 0x17];
const DEFAULTS_MARKER: &[u8] = &[0xCB, 0x0F, 0xB4, 0xB0, 0xE0];

/// Walk every CF E0 marker in the payload and return its 6-byte tail
/// (the `E000`-prefixed GUID identifier). Used for the 51 references that
/// appear in every discipline payload.
fn extract_cf_e0_tails(payload: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 9 <= payload.len() {
        if payload[i] == 0xCF
            && payload[i + 1] == 0xE0
            && payload[i + 2] == 0x00
            && i + 9 <= payload.len()
        {
            out.push(&payload[i + 3..i + 9]);
            i += 9;
        } else {
            i += 1;
        }
    }
    out
}

fn tail_to_guid_hex(tail: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    s.push_str("E000");
    for b in tail {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn extract_codename(payload: &[u8]) -> Result<String> {
    let class_pos = find_subsequence(payload, CLASS_MARKER)
        .ok_or_else(|| anyhow::anyhow!("dis payload missing class marker CF40 41FC3C7A20"))?;
    let after = class_pos + CLASS_MARKER.len();
    if after + 2 > payload.len() || payload[after] != 0x06 {
        bail!("dis payload missing 06 length-prefix after class marker");
    }
    let len = payload[after + 1] as usize;
    let start = after + 2;
    let end = start + len;
    if end > payload.len() {
        bail!("dis codename length {len} extends past payload end");
    }
    std::str::from_utf8(&payload[start..end])
        .map(|s| s.to_string())
        .map_err(|e| anyhow::anyhow!("dis codename not utf8: {e}"))
}

fn extract_tiers(payload: &[u8]) -> Result<Vec<DisciplineTier>> {
    // Tier records sit immediately after the TIER_ARRAY_OPENER. Each tier is
    // 10 bytes:
    //   02 03 03 [01 a] [02 b] [03 c] [level]
    // 8 tiers expected. The last tier's level byte slot is sometimes followed
    // directly by an array-terminator sequence; that's part of the same 10
    // bytes so the loop is uniform.
    let opener_pos = find_subsequence(payload, TIER_ARRAY_OPENER)
        .ok_or_else(|| anyhow::anyhow!("dis payload missing tier array opener"))?;
    let mut cursor = opener_pos + TIER_ARRAY_OPENER.len();
    let mut tiers = Vec::with_capacity(8);
    for ordinal in 1..=8u8 {
        if cursor + 10 > payload.len() {
            bail!(
                "dis payload truncated at tier {ordinal}; cursor={cursor} len={}",
                payload.len()
            );
        }
        let record = &payload[cursor..cursor + 10];
        if record[0..3] != [0x02, 0x03, 0x03] {
            bail!(
                "dis payload: expected tier header 02 03 03 at offset {cursor}, got {:02X?}",
                &record[0..3]
            );
        }
        // Layout: 02 03 03 [01 a] [02 b] [03 c] [level]
        let a = record[4];
        let b = record[6];
        let c = record[8];
        let level = record[9];
        tiers.push(DisciplineTier {
            ordinal,
            level,
            choice_indices: [a, b, c],
        });
        cursor += 10;
    }
    Ok(tiers)
}

fn extract_index_to_level(payload: &[u8]) -> Result<[u8; 24]> {
    // The index->level map is found via the CA B1 00 7D marker; the
    // payload then has a 9-byte CE header (`02 CE 0B FC 49 00 00 01 <ver>`),
    // a 4-byte CA 48 EF E0 sub-marker, and a 5-byte `08 02 02 18 18` array
    // opener (for 24 entries) before the 24 (index, level) byte pairs begin.
    // Total skip: 4 (CA B1 marker) + 9 + 4 + 5 = 22 bytes.
    let marker = [0xCA, 0xB1, 0x00, 0x7D];
    let marker_pos = find_subsequence(payload, &marker)
        .ok_or_else(|| anyhow::anyhow!("dis payload missing index-level marker"))?;
    let pairs_start = marker_pos + 22;
    let pairs_end = pairs_start + 48;
    if pairs_end > payload.len() {
        bail!(
            "dis payload truncated at index-level map; need {pairs_end} bytes, have {}",
            payload.len()
        );
    }
    let mut levels = [0u8; 24];
    for i in 0..24 {
        let idx = payload[pairs_start + i * 2];
        let lvl = payload[pairs_start + i * 2 + 1];
        let expected_idx = (i + 1) as u8;
        if idx != expected_idx {
            bail!("dis index-level pair {i}: expected index {expected_idx}, got {idx}");
        }
        levels[i] = lvl;
    }
    Ok(levels)
}

fn extract_defaults(payload: &[u8]) -> Result<Vec<DisciplineDefault>> {
    let marker_pos = find_subsequence(payload, DEFAULTS_MARKER)
        .ok_or_else(|| anyhow::anyhow!("dis payload missing defaults marker"))?;
    // After CB 0F B4 B0 E0: 6-byte array opener (08 02 02 08 08 17), then
    // 8 (level, index) pairs.
    let pairs_start = marker_pos + 11;
    let pairs_end = pairs_start + 16;
    if pairs_end > payload.len() {
        bail!(
            "dis payload truncated at defaults map; need {pairs_end} bytes, have {}",
            payload.len()
        );
    }
    let mut defaults = Vec::with_capacity(8);
    for i in 0..8 {
        let level = payload[pairs_start + i * 2];
        let default_mod_index = payload[pairs_start + i * 2 + 1];
        defaults.push(DisciplineDefault {
            level,
            default_mod_index,
        });
    }
    Ok(defaults)
}

/// Decode a `dis.*` PBUK payload.
///
/// Format invariant across all 48 sampled disciplines (verified by
/// `kessel-discovery/src/bin/dis_format_audit.rs`):
///   - exactly 51 CF E0 markers (2 apc + 24 main mod list + 24 sorted lookup + 1 signature)
///   - exactly 8 tier triplets
///   - one trailing signature ability
pub fn decode_dis_payload(payload: &[u8]) -> Result<DisciplineRecord> {
    let codename = extract_codename(payload)?;
    let cf_e0_tails = extract_cf_e0_tails(payload);
    if cf_e0_tails.len() != 51 {
        bail!(
            "dis payload: expected 51 CF E0 markers, got {}",
            cf_e0_tails.len()
        );
    }

    // Layout per probe doc:
    //   index 0: icon apc
    //   index 1: mod tree apc
    //   indices 2..26: 24 main mod entries (declaration order)
    //   indices 26..50: 24 sorted lookup duplicates (skipped)
    //   index 50: signature ability
    let icon_apc_guid = tail_to_guid_hex(cf_e0_tails[0]);
    let mod_tree_apc_guid = tail_to_guid_hex(cf_e0_tails[1]);
    let signature_ability_guid = tail_to_guid_hex(cf_e0_tails[50]);

    let tiers = extract_tiers(payload)?;
    let levels = extract_index_to_level(payload)?;
    let defaults = extract_defaults(payload)?;

    let mut mods = Vec::with_capacity(24);
    for (i, tail) in cf_e0_tails[2..26].iter().enumerate() {
        mods.push(DisciplineModEntry {
            index: (i + 1) as u8,
            guid: tail_to_guid_hex(tail),
            level: levels[i],
        });
    }

    Ok(DisciplineRecord {
        codename,
        icon_apc_guid,
        mod_tree_apc_guid,
        mods,
        tiers,
        defaults,
        signature_ability_guid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real bytes of dis.powertech.firebug pulled via probe_dis 2026-05-24.
    fn firebug_payload() -> Vec<u8> {
        // 813 bytes. Captured into a fixture and embedded as a hex literal.
        // Generated by:
        //   ./target/release/probe_dis -i ~/swtor/Assets -f dis.powertech.firebug
        // Lines wrapped for readability; the function strips whitespace.
        let hex = "\
            00 00 00 00 00 00 00 11 10 CF 40 00 00 41 FC 3C 7A 20 06 0E \
            70 6F 77 65 72 5F 70 79 72 6F 74 65 63 68 CC 02 BC B7 16 50 \
            01 CE 0B FC 49 00 00 01 30 CB 3C 33 FC C0 05 03 CC 08 7C FF \
            1F EB 05 04 CC 02 14 9C B9 71 02 CE 07 37 7E 00 00 00 22 01 \
            02 CE 07 37 7E 00 00 00 23 01 07 01 01 01 01 CF E0 00 68 A7 \
            08 70 33 E1 01 01 CF E0 00 FE 2E FC 9D 17 DA CD 03 82 C8 2C \
            11 31 03 01 01 08 02 07 08 08 17 02 03 03 01 01 02 03 03 02 \
            1B 02 03 03 01 04 02 06 03 05 27 02 03 03 01 08 02 09 03 07 \
            2B 02 03 03 01 0C 02 0A 03 0B 33 02 03 03 01 0F 02 0E 03 0D \
            40 02 03 03 01 11 02 12 03 10 44 02 03 03 01 15 02 13 03 14 \
            49 02 03 03 01 16 02 18 03 17 01 08 02 01 18 18 01 CF E0 00 \
            7B 91 F9 B5 C2 78 02 CF E0 00 B4 71 39 13 39 DA 03 CF E0 00 \
            91 FC 5C AA 72 39 04 CF E0 00 13 F8 12 94 70 64 05 CF E0 00 \
            2A 8E 45 6E E2 B7 06 CF E0 00 22 85 1F 0C AD 9B 07 CF E0 00 \
            C5 0C AF 60 C7 F9 08 CF E0 00 67 C6 C5 5C 81 7C 09 CF E0 00 \
            8A D0 F2 BE A9 07 0A CF E0 00 8B 07 4F 96 ED 6F 0B CF E0 00 \
            F7 FF D5 9A C4 39 0C CF E0 00 34 09 44 1D 41 AD 0D CF E0 00 \
            CA CD 4A A1 01 EC 0E CF E0 00 94 1E F1 2D 61 2E 0F CF E0 00 \
            3E A4 3B 4C AE E8 10 CF E0 00 ED 6A 2D B0 14 8A 11 CF E0 00 \
            4E 67 B0 0D 96 8A 12 CF E0 00 5B DE 7B 44 D6 46 13 CF E0 00 \
            40 66 99 E6 73 99 14 CF E0 00 CE E5 B9 8D CC 26 15 CF E0 00 \
            17 73 6D 8F C6 F2 16 CF E0 00 91 23 0A 23 67 72 17 CF E0 00 \
            B7 91 23 D7 25 D2 18 CF E0 00 9F 2A 2D 89 E5 27 01 08 01 02 \
            18 18 CF E0 00 13 F8 12 94 70 64 04 CF E0 00 17 73 6D 8F C6 \
            F2 15 CF E0 00 22 85 1F 0C AD 9B 06 CF E0 00 2A 8E 45 6E E2 \
            B7 05 CF E0 00 34 09 44 1D 41 AD 0C CF E0 00 3E A4 3B 4C AE \
            E8 0F CF E0 00 40 66 99 E6 73 99 13 CF E0 00 4E 67 B0 0D 96 \
            8A 11 CF E0 00 5B DE 7B 44 D6 46 12 CF E0 00 67 C6 C5 5C 81 \
            7C 08 CF E0 00 7B 91 F9 B5 C2 78 01 CF E0 00 8A D0 F2 BE A9 \
            07 09 CF E0 00 8B 07 4F 96 ED 6F 0A CF E0 00 91 23 0A 23 67 \
            72 16 CF E0 00 91 FC 5C AA 72 39 03 CF E0 00 94 1E F1 2D 61 \
            2E 0E CF E0 00 9F 2A 2D 89 E5 27 18 CF E0 00 B4 71 39 13 39 \
            DA 02 CF E0 00 B7 91 23 D7 25 D2 17 CF E0 00 C5 0C AF 60 C7 \
            F9 07 CF E0 00 CA CD 4A A1 01 EC 0D CF E0 00 CE E5 B9 8D CC \
            26 14 CF E0 00 ED 6A 2D B0 14 8A 10 CF E0 00 F7 FF D5 9A C4 \
            39 0B CA B1 00 7D 02 CE 0B FC 49 00 00 01 04 CA 48 EF E0 08 \
            02 02 18 18 01 17 02 17 03 17 04 1B 05 1B 06 1B 07 27 08 27 \
            09 27 0A 2B 0B 2B 0C 2B 0D 33 0E 33 0F 33 10 40 11 40 12 40 \
            13 44 14 44 15 44 16 49 17 49 18 49 CB 0F B4 B0 E0 08 02 02 \
            08 08 17 02 1B 05 27 09 2B 0A 33 0F 40 12 44 13 49 18 01 07 \
            01 01 01 01 CF E0 00 93 4F D2 6C 51 95";
        hex_to_bytes(hex)
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|s| u8::from_str_radix(s, 16).expect("hex"))
            .collect()
    }

    #[test]
    fn extract_codename_finds_firebug() {
        let payload = firebug_payload();
        assert_eq!(extract_codename(&payload).unwrap(), "power_pyrotech");
    }

    #[test]
    fn decode_dis_payload_firebug_top_level() {
        let payload = firebug_payload();
        let record = decode_dis_payload(&payload).expect("decode");
        assert_eq!(record.codename, "power_pyrotech");
        assert_eq!(record.icon_apc_guid, "E00068A7087033E1");
        assert_eq!(record.mod_tree_apc_guid, "E000FE2EFC9D17DA");
        assert_eq!(record.signature_ability_guid, "E000934FD26C5195"); // flaming_fist
        assert_eq!(record.mods.len(), 24);
        assert_eq!(record.tiers.len(), 8);
        assert_eq!(record.defaults.len(), 8);
    }

    #[test]
    fn decode_dis_payload_firebug_first_tier_is_primed_ignition_choice() {
        let payload = firebug_payload();
        let record = decode_dis_payload(&payload).unwrap();
        // Tier 1 in-game: (Primed Ignition, Open Flame, Heatstroke) at level 27
        let tier1 = &record.tiers[0];
        assert_eq!(tier1.ordinal, 1);
        assert_eq!(tier1.level, 0x1B);
        assert_eq!(tier1.choice_indices, [1, 3, 2]);
    }

    #[test]
    fn decode_dis_payload_firebug_mod_levels_match_index_map() {
        let payload = firebug_payload();
        let record = decode_dis_payload(&payload).unwrap();
        // Indices 01-03 at level 0x17 (23); 04-06 at 0x1B (27); ...
        // First three:
        for (i, expected_level) in [0x17, 0x17, 0x17].iter().enumerate() {
            assert_eq!(record.mods[i].level, *expected_level);
        }
        // Indices 22-24 (positions 21-23 in 0-based) at level 0x49 (73)
        for i in 21..24 {
            assert_eq!(record.mods[i].level, 0x49);
        }
    }

    #[test]
    fn decode_dis_payload_firebug_first_mod_is_primed_ignition() {
        let payload = firebug_payload();
        let record = decode_dis_payload(&payload).unwrap();
        // Index 1 = primed_ignition = E0007B91F9B5C278
        assert_eq!(record.mods[0].index, 1);
        assert_eq!(record.mods[0].guid, "E0007B91F9B5C278");
    }

    #[test]
    fn decode_dis_payload_rejects_payload_with_wrong_cf_e0_count() {
        // Truncate the payload so we only have a partial set of CF E0 markers.
        let payload = firebug_payload();
        let truncated = &payload[..100];
        let err = decode_dis_payload(truncated).unwrap_err();
        assert!(
            err.to_string().contains("expected 51 CF E0 markers"),
            "wrong error: {err}"
        );
    }
}
