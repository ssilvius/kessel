//! GSF talent stat decoder.
//!
//! `tal.spvp.*` payloads encode numeric stat values as records of the form:
//!
//! ```text
//! ...01 04 <f32 LE>... cb 19 d7 4b ?? 03 ...
//!     ^^^^^           ^^^^^^^^^^^^^^^^^^
//!     value-type      record-end signature
//! ```
//!
//! The single byte immediately before `01 04` is the stat ID. Two byte-level
//! variants exist for the lead-in:
//!
//! - canonical: `c9 01 <stat_id> 01 04 <f32 LE>`
//! - bare:      `<stat_id> 01 04 <f32 LE>`
//!
//! Both flow into the same record-end signature `cb 19 d7 4b ?? 03`, where
//! the fifth byte alternates (`9d` for second/last record, `96` for an
//! intermediate record). Walking back from the signature handles every
//! encoded record without needing a sentinel like ground abilities use.
//!
//! Discovered empirically on the v7.8.1 spice extraction: 250/350 GSF
//! talents (71%) carry at least one record; the remaining 100 are flag-only
//! talents whose effects live on the parent ability or in script hooks.
//!
//! Stat-ID semantics are deliberately NOT hard-coded here -- the byte is
//! exposed as-is and the dictionary lives downstream. Adding more semantic
//! mappings later does not require re-parsing payloads.
//!
//! Verified validations against in-game descriptions:
//! - tal.spvp.crew.offensive.firing_arc        ("+2 deg") -> 0x5f = 2.0
//! - tal.spvp.shield.shield_projector.tier1    ("-10 s")  -> 0x40 = -10.0
//! - tal.spvp.engine.tensor_field.tier3        ("+4 s")   -> 0x41 = 4.0
//! - tal.spvp.minor_thrusters.engine_power_regen.upgrade  -> 0x22/0x24
//!   pairs at 0.04/0.08/0.12 (matches "+4%/+8%/+12%" rank text)

/// Constant prefix of the record-end signature `cb 19 d7 4b ?? 03`.
const SIG_PREFIX: [u8; 4] = [0xCB, 0x19, 0xD7, 0x4B];

/// One stat record decoded from a talent payload.
#[derive(Debug, Clone, PartialEq)]
pub struct GsfStatRecord {
    /// Order within the payload (0-indexed by byte offset). Multiple records
    /// with the same `stat_id` are common -- they encode rank progressions
    /// or per-effect duplicates -- and ordinal preserves their order.
    pub ordinal: u32,
    /// Single-byte stat identifier (e.g., 0x40 cooldown delta, 0x5f firing
    /// arc degrees). The full ID dictionary is documented in
    /// `gsf_stat_dictionary.toml` and exposed via `gsf_stat_dictionary`.
    pub stat_id: u8,
    /// Decoded f32 value, units defined by the stat ID. Negative values are
    /// real (cooldown reductions, lock-on time decreases, etc).
    pub value: f32,
}

/// Decode every GSF stat record in a `tal.spvp.*` payload.
///
/// Anchors on the record-end signature `cb 19 d7 4b ?? 03`, then walks back
/// to recover the f32 value and stat ID. Skips records whose value is not
/// finite or is an exact zero (uninit). Returns records in payload order.
pub fn decode_gsf_stats(payload: &[u8]) -> Vec<GsfStatRecord> {
    let mut out = Vec::new();
    let mut ordinal: u32 = 0;
    let mut i = 0;
    while i + 6 <= payload.len() {
        // Match cb 19 d7 4b ?? 03 with a wildcard on byte 4.
        if payload[i..i + 4] != SIG_PREFIX || payload[i + 5] != 0x03 {
            i += 1;
            continue;
        }
        // The 01 04 marker can sit immediately before the signature, or be
        // separated by a 3-4 byte trailer such as `07 05` or `07 05 06`.
        let mut decoded = None;
        for back in [6usize, 9, 10] {
            if i < back + 1 {
                continue;
            }
            let marker = i - back;
            if payload[marker] != 0x01 || payload[marker + 1] != 0x04 {
                continue;
            }
            let val_bytes: [u8; 4] = payload[marker + 2..marker + 6]
                .try_into()
                .expect("4-byte slice");
            let value = f32::from_le_bytes(val_bytes);
            if !value.is_finite() || value == 0.0 {
                break;
            }
            let stat_id = payload[marker - 1];
            decoded = Some(GsfStatRecord {
                ordinal,
                stat_id,
                value,
            });
            break;
        }
        if let Some(rec) = decoded {
            out.push(rec);
            ordinal += 1;
        }
        i += 6;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(stat_id: u8, value: f32, with_c9: bool, sig5: u8) -> Vec<u8> {
        let mut buf = vec![0xAAu8; 16];
        if with_c9 {
            buf.extend_from_slice(&[0xC9, 0x01]);
        }
        buf.push(stat_id);
        buf.extend_from_slice(&[0x01, 0x04]);
        buf.extend_from_slice(&value.to_le_bytes());
        buf.extend_from_slice(&[0xCB, 0x19, 0xD7, 0x4B, sig5, 0x03, 0x01, 0x01]);
        buf
    }

    #[test]
    fn decodes_canonical_c9_record() {
        let payload = build(0x5F, 2.0, true, 0x9D);
        let recs = decode_gsf_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].stat_id, 0x5F);
        assert_eq!(recs[0].value, 2.0);
        assert_eq!(recs[0].ordinal, 0);
    }

    #[test]
    fn decodes_bare_record_without_c9_prefix() {
        let payload = build(0x40, -10.0, false, 0x9D);
        let recs = decode_gsf_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].stat_id, 0x40);
        assert_eq!(recs[0].value, -10.0);
    }

    #[test]
    fn handles_intermediate_signature_byte_96() {
        let payload = build(0x62, 0.30, true, 0x96);
        let recs = decode_gsf_stats(&payload);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].stat_id, 0x62);
        assert!((recs[0].value - 0.30).abs() < 1e-6);
    }

    #[test]
    fn decodes_multiple_records_in_order() {
        let mut payload = build(0x5F, 1.0, true, 0x9D);
        payload.extend_from_slice(&build(0x4F, 0.05, true, 0x9D));
        let recs = decode_gsf_stats(&payload);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].stat_id, 0x5F);
        assert_eq!(recs[0].ordinal, 0);
        assert_eq!(recs[1].stat_id, 0x4F);
        assert_eq!(recs[1].ordinal, 1);
        assert!((recs[1].value - 0.05).abs() < 1e-6);
    }

    #[test]
    fn rejects_zero_value_records() {
        let payload = build(0x40, 0.0, true, 0x9D);
        assert!(decode_gsf_stats(&payload).is_empty());
    }

    #[test]
    fn returns_empty_when_no_signature_present() {
        let payload = vec![0u8; 64];
        assert!(decode_gsf_stats(&payload).is_empty());
    }

    #[test]
    fn handles_trailer_bytes_between_value_and_signature() {
        // magazine_capacity-style: 01 04 <f32> 07 05 06 cb 19 d7 4b 96 03
        let mut buf = vec![0xAAu8; 16];
        buf.extend_from_slice(&[0xC9, 0x01, 0x62]);
        buf.extend_from_slice(&[0x01, 0x04]);
        buf.extend_from_slice(&0.30f32.to_le_bytes());
        buf.extend_from_slice(&[0x07, 0x05, 0x06]);
        buf.extend_from_slice(&[0xCB, 0x19, 0xD7, 0x4B, 0x96, 0x03, 0x01, 0x01]);
        let recs = decode_gsf_stats(&buf);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].stat_id, 0x62);
        assert!((recs[0].value - 0.30).abs() < 1e-6);
    }
}
