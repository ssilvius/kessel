//! GSF base ability stat decoder.
//!
//! `abl.spvp.*` payloads carry numeric stats in a GOM token stream. The
//! grammar relevant to stat extraction:
//!
//! - `<prop_id u8> 04 <f32 LE>` -- a typed-f32 property record. `prop_id` is
//!   a 1-byte field index; `04` is the f32 type tag. This is the only token
//!   form `decode_gsf_ability_stats` emits.
//! - `01 06 <len u8> <ascii>` -- typed-string token (the ability's FQN
//!   string). Consumed so the ASCII payload isn't rescanned for stats.
//! - `CE 0B <2 bytes> 04 00 00 01 <byte>` -- CE-content-ref into a localized
//!   rank-text table. The embedded `04` would otherwise be misread as an
//!   f32 type tag.
//! - `CF 40 <8 bytes>` -- template-GUID reference.
//! - `CF E0 <6 bytes>` -- content-GUID reference. Misreads also happen when
//!   the f32-payload bytes of a candidate property happen to start with
//!   `CF E0 00`; we look ahead and skip in that case.
//! - `C9 <2 bytes BE>` -- big-endian u16 token (used in some prototype
//!   contexts; benign here but consumed).
//!
//! The walker tries known opaque tokens first (longest-prefix wins) so their
//! internal bytes never reach the property-record matcher. This eliminates
//! the false-positive prop_ids (0x09, 0x35, 0x3E, 0x51, 0x5D, 0xBA) that the
//! previous window scanner used to emit by reading CE/CF token internals as
//! f32 records.
//!
//! Coverage on the v7.8.1 spice extraction: 113/131 base GSF abilities (86%)
//! emit at least one record. The 18 uncovered abilities are passive auras
//! whose effects live on a parent activator or in script hooks.
//!
//! Verified anchors:
//! - `abl.spvp.engine.barrel_roll`  -> 0x0402 = 30.0  (30s cooldown)
//! - `abl.spvp.engine.power_dive`   -> 0x0402 = 15.0  (15s cooldown)
//!
//! Stat-ID semantics differ from ground abilities (e.g. `0x0402` is the
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

/// Decode every f32 property record in an `abl.spvp.*` payload, skipping
/// known opaque GOM tokens so their internal bytes are never misread.
pub fn decode_gsf_ability_stats(payload: &[u8]) -> Vec<GsfAbilityStatRecord> {
    let mut out = Vec::new();
    let mut ordinal: u32 = 0;
    let mut i = 0;
    while i < payload.len() {
        if let Some(consumed) = try_consume_opaque(payload, i) {
            i += consumed;
            continue;
        }
        if let Some((prop_id, value)) = try_match_property(payload, i) {
            out.push(GsfAbilityStatRecord {
                ordinal,
                prop_id,
                value,
            });
            ordinal += 1;
            i += 6;
            continue;
        }
        i += 1;
    }
    out
}

/// Try to consume a known opaque GOM token at `i`. Returns the byte count
/// consumed when matched.
fn try_consume_opaque(payload: &[u8], i: usize) -> Option<usize> {
    let rest = payload.get(i..)?;
    match rest.first()? {
        // Typed string: `01 06 <len> <ascii>`
        0x01 if rest.get(1) == Some(&0x06) => {
            let len = *rest.get(2)? as usize;
            let end = 3 + len;
            if end <= rest.len() && rest[3..end].iter().all(|&b| (32..127).contains(&b)) {
                return Some(end);
            }
            None
        }
        // CE-content-ref into rank-text table:
        //   `CE 0B <rank_byte> 04 00 00 01 <rank_idx>` (8 bytes).
        // The `04` at position 3 would otherwise look like an f32 type tag.
        0xCE if rest.get(1) == Some(&0x0B)
            && rest.get(3) == Some(&0x04)
            && rest.get(4) == Some(&0x00)
            && rest.get(5) == Some(&0x00)
            && rest.get(6) == Some(&0x01)
            && rest.len() >= 8 =>
        {
            Some(8)
        }
        // Template GUID: `CF 40 <8 bytes>`.
        0xCF if rest.get(1) == Some(&0x40) && rest.len() >= 10 => Some(10),
        // Content GUID: `CF E0 <6 bytes>`.
        0xCF if rest.get(1) == Some(&0xE0) && rest.len() >= 8 => Some(8),
        _ => None,
    }
}

/// True if the 2-byte sequence at `rest[off..off+2]` is the start of a known
/// opaque GOM token. Used to detect misalignments where a candidate property
/// record's f32 payload overlaps with a real token.
fn starts_opaque_token(rest: &[u8], off: usize) -> bool {
    let Some(a) = rest.get(off) else { return false };
    let Some(b) = rest.get(off + 1) else { return false };
    matches!((a, b), (0xCE, 0x0B) | (0xCF, 0x40) | (0xCF, 0xE0))
}

/// Try to match an f32 property record `<prop_id u8> 04 <f32 LE>` at `i`.
/// Returns `(prop_id_u16, value)` when the bytes form a valid record.
///
/// Rejects matches where the f32 payload looks like the start of a CF-E0
/// content-guid (`CF E0 00 ?`) -- that's a misalignment, not a stat.
fn try_match_property(payload: &[u8], i: usize) -> Option<(u16, f32)> {
    let rest = payload.get(i..)?;
    if rest.len() < 6 {
        return None;
    }
    if rest[1] != 0x04 {
        return None;
    }
    // Reject f32 payloads that overlap the start of a known opaque token.
    // Misalignments at offsets 0 or 1 of the f32 (record offsets 2..4) can
    // still yield in-range f32 values; at offsets 2 or 3 the exponent byte
    // falls into the token bytes and the magnitude filter rejects naturally.
    if starts_opaque_token(rest, 2) || starts_opaque_token(rest, 3) {
        return None;
    }
    let value = f32::from_le_bytes([rest[2], rest[3], rest[4], rest[5]]);
    if !value.is_finite() || value == 0.0 {
        return None;
    }
    let mag = value.abs();
    if !(MIN_MAGNITUDE..=MAX_MAGNITUDE).contains(&mag) {
        return None;
    }
    let prop_id = u16::from_le_bytes([rest[0], rest[1]]);
    Some((prop_id, value))
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
        // barrel_roll-like: noise + cooldown record + noise.
        let mut payload = vec![0xAAu8; 35];
        payload.extend_from_slice(&rec(0x0402, 30.0));
        payload.extend_from_slice(&[0xCB, 0x51, 0x3F, 0xE5, 0x41]);
        let recs = decode_gsf_ability_stats(&payload);
        assert!(recs.iter().any(|r| r.prop_id == 0x0402 && r.value == 30.0));
    }

    #[test]
    fn rejects_subnormal_magnitudes() {
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
    fn rejects_huge_magnitudes() {
        let payload = rec(0x0400, -536576.125);
        assert!(decode_gsf_ability_stats(&payload).is_empty());
    }

    #[test]
    fn skips_non_0x04_high_byte() {
        let mut payload = rec(0x0502, 30.0);
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
        let payload = rec(0x0402, -1.0);
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].value, -1.0);
    }

    #[test]
    fn skips_ce_content_ref_token() {
        // `CE 0B BA 04 00 00 01 XX` is a CE-content-ref into a rank-text
        // table. The embedded `04` previously misread as an f32 type tag,
        // emitting bogus prop_id=0x04BA records (the spurious "12-rank
        // ladder" on abl.spvp.systems.aoe_defensive_boost).
        let mut payload = vec![0xCE, 0x0B, 0xBA, 0x04, 0x00, 0x00, 0x01, 0x05];
        payload.extend_from_slice(&rec(0x0402, 30.0));
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1, "only the real cooldown should emit");
        assert_eq!(recs[0].prop_id, 0x0402);
    }

    #[test]
    fn skips_ce_0b_in_f32_payload_misalignment() {
        // `09 04 04 CE 0B C6 ...` -- f32 payload bytes 1-2 are `CE 0B`, the
        // start of a CE-content-ref token at offset +3 of the record. Real
        // example from 9 crew/drone abilities producing the bogus 0x0409
        // ~-8947 / -559 values.
        let payload = vec![0x09, 0x04, 0x04, 0xCE, 0x0B, 0xC6, 0xAA, 0xBB];
        let recs = decode_gsf_ability_stats(&payload);
        assert!(
            recs.iter().all(|r| r.prop_id != 0x0409),
            "CE-0B-in-f32 misalignment must not emit a bogus 0x0409 stat",
        );
    }

    #[test]
    fn skips_cf_e0_content_guid_misalignment() {
        // `XX 04 CF E0 00 YY ZZ ZZ ZZ` -- the f32 payload bytes are the
        // start of a CF-E0 content-guid token, not a value. Real example
        // from abl.spvp.missile.cluster_missiles (the bogus 0x0435 = -32.22).
        let payload = vec![0x35, 0x04, 0xCF, 0xE0, 0x00, 0xC2, 0xAA, 0xBB];
        let recs = decode_gsf_ability_stats(&payload);
        assert!(
            recs.iter().all(|r| r.prop_id != 0x0435),
            "CF-E0 misalignment must not emit a bogus 0x0435 stat",
        );
    }

    #[test]
    fn consumes_typed_string_without_rescanning() {
        // `01 06 <len> <ascii>` is the FQN string token. Its ASCII payload
        // must not be rescanned for f32 records (defence-in-depth; ASCII
        // rarely contains 0x04).
        let mut payload = vec![0x01, 0x06, 0x0F];
        payload.extend_from_slice(b"spvp_barrelroll");
        payload.extend_from_slice(&rec(0x0402, 30.0));
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].prop_id, 0x0402);
    }

    #[test]
    fn consumes_template_and_content_guids() {
        // `CF 40 + 8 bytes` and `CF E0 + 6 bytes` tokens.
        let mut payload = vec![0xCF, 0x40];
        payload.extend(std::iter::repeat(0xAA).take(8));
        payload.push(0xCF);
        payload.push(0xE0);
        payload.extend(std::iter::repeat(0xBB).take(6));
        payload.extend_from_slice(&rec(0x0402, 30.0));
        let recs = decode_gsf_ability_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].prop_id, 0x0402);
    }
}
