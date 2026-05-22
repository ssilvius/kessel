//! GSF base ability stat decoder.
//!
//! `abl.spvp.*` payloads encode numeric stats in the same `[u16 LE prop_id]
//! [f32 LE value]` layout used by ground abilities, but without the
//! `01 04 00 00 80 BF` (-1.0 cooldown init) sentinel that anchors
//! `scan_ability_props`. Records are scattered across the payload rather than
//! packed into a single contiguous block, so the decoder walks every 6-byte
//! window and emits any window where:
//!
//! - the high byte of `prop_id` is `0x04` (the universal ability-prop class)
//! - the value is finite and non-zero
//! - `|value| >= 0.01` (subnormal-ish magnitudes are byte-alignment noise)
//! - `|value| <= 100_000.0` (huge magnitudes are GUID-byte coincidences;
//!   real GSF stats are seconds, percents, meters -- all well under 100k)
//!
//! Coverage on the v7.8.1 spice extraction: 113/131 base GSF abilities (86%)
//! emit at least one record. The 18 uncovered abilities are passive auras
//! whose effects live on a parent activator or in script hooks.
//!
//! Verified anchors (matching huttspawn's manual GSF corrections):
//! - `abl.spvp.engine.barrel_roll`  -> 0x0402 = 30.0  (30s cooldown)
//! - `abl.spvp.engine.power_dive`   -> 0x0402 = 15.0  (15s cooldown)
//!
//! Stat-ID semantics differ from ground abilities (e.g., `0x0402` is the
//! cooldown for GSF, an animation marker for ground). Downstream consumers
//! pivot on the wide-format `gsf_ability_stats` table; no semantic mapping
//! is hard-coded here.

/// One stat record decoded from an `abl.spvp.*` payload.
#[derive(Debug, Clone, PartialEq)]
pub struct GsfAbilityStatRecord {
    /// Order within the payload (0-indexed by emission order).
    pub ordinal: u32,
    /// Two-byte stat identifier (high byte always `0x04`). Exposed verbatim;
    /// downstream tables interpret semantics.
    pub prop_id: u16,
    /// Decoded f32 value. Negative values are real (init markers, refunds).
    pub value: f32,
}

const MIN_MAGNITUDE: f32 = 0.01;
const MAX_MAGNITUDE: f32 = 100_000.0;

/// Decode every plausible 0x04xx stat record in an `abl.spvp.*` payload.
pub fn decode_gsf_ability_stats(payload: &[u8]) -> Vec<GsfAbilityStatRecord> {
    let mut out = Vec::new();
    let mut ordinal: u32 = 0;
    let mut i = 0;
    while i + 6 <= payload.len() {
        if payload[i + 1] != 0x04 {
            i += 1;
            continue;
        }
        let value = f32::from_le_bytes([
            payload[i + 2],
            payload[i + 3],
            payload[i + 4],
            payload[i + 5],
        ]);
        if !value.is_finite() || value == 0.0 {
            i += 1;
            continue;
        }
        let mag = value.abs();
        if !(MIN_MAGNITUDE..=MAX_MAGNITUDE).contains(&mag) {
            i += 1;
            continue;
        }
        let prop_id = u16::from_le_bytes([payload[i], payload[i + 1]]);
        out.push(GsfAbilityStatRecord {
            ordinal,
            prop_id,
            value,
        });
        ordinal += 1;
        i += 6;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(prop_id: u16, value: f32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(6);
        buf.extend_from_slice(&prop_id.to_le_bytes());
        buf.extend_from_slice(&value.to_le_bytes());
        buf
    }

    #[test]
    fn decodes_isolated_cooldown_record() {
        // Mimic barrel_roll: noise + cooldown record + noise.
        let mut payload = vec![0xAAu8; 35];
        payload.extend_from_slice(&rec(0x0402, 30.0));
        payload.extend_from_slice(&[0xCB, 0x51, 0x3F, 0xE5, 0x41]);
        let recs = decode_gsf_ability_stats(&payload);
        assert!(recs.iter().any(|r| r.prop_id == 0x0402 && r.value == 30.0));
    }

    #[test]
    fn rejects_subnormal_magnitudes() {
        // Garbage byte alignment that looks like a 0x04xx record.
        let payload = rec(0x0400, -0.006387935);
        assert!(decode_gsf_ability_stats(&payload).is_empty());
    }

    #[test]
    fn rejects_zero_and_non_finite() {
        let mut payload = rec(0x0402, 0.0);
        payload.extend_from_slice(&rec(0x0402, f32::INFINITY));
        payload.extend_from_slice(&rec(0x0402, f32::NAN));
        assert!(decode_gsf_ability_stats(&payload).is_empty());
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn rejects_huge_magnitudes() {
        let payload = rec(0x0400, -536576.125);
        assert!(decode_gsf_ability_stats(&payload).is_empty());
    }

    #[test]
    fn skips_non_0x04_high_byte() {
        let mut payload = rec(0x0502, 30.0); // wrong class
        payload.extend_from_slice(&rec(0x0402, 30.0));
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].prop_id, 0x0402);
    }

    #[test]
    fn ordinal_increments_in_emission_order() {
        let mut payload = vec![];
        payload.extend_from_slice(&rec(0x0402, 60.0));
        payload.extend_from_slice(&[0xAA, 0xAA]);
        payload.extend_from_slice(&rec(0x0421, 30.0));
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].ordinal, 0);
        assert_eq!(recs[0].prop_id, 0x0402);
        assert_eq!(recs[1].ordinal, 1);
        assert_eq!(recs[1].prop_id, 0x0421);
    }

    #[test]
    fn keeps_negative_one_passive_marker() {
        // GSF passive auras emit 0x0402 = -1.0 (no cooldown). This is real
        // signal and must not be filtered as garbage.
        let payload = rec(0x0402, -1.0);
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].value, -1.0);
    }
}
