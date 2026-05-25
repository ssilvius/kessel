//! Decoder for the indexed effect-block CF E0 sequence in ability/talent
//! payloads. Issue #173.
//!
//! Each ability/talent payload contains (after the standard header + icon +
//! primitive-property records) a series of `CF E0 00 <8-byte GUID>` markers.
//! The first marker is typically the parent's OWN content GUID (a self-ref).
//! The next N markers are PRECEDED by a 1-byte index (01..N) and reference
//! the parent's effect-block sub-records. For Massacre: parent self-ref +
//! 4 indexed effect blocks (indices 1..4).
//!
//! Some referenced effect-block GUIDs do not resolve in the `objects` table
//! because their underlying abilities exist only as versioned variants
//! (Heat Chain category, issue #179). The decoder preserves the raw GUID so
//! the unresolved edges are visible in spice rather than silently dropped.
//!
//! What this decoder does NOT do: extract the typed properties (Weapon
//! Damage Coefficient + Standard Health Percent, Modify Meta Stat amount
//! and stat ref, Play Appearance epp ref, Call Effect chain metadata). Per
//! the parsely reference shape, those fields live inside the per-effect-
//! block sub-record payloads, and decoding them requires per-property
//! byte-layout work that is filed as a follow-on. This decoder captures the
//! structural linkage so consumers can at least walk the parent → effect
//! block graph.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectBlockRef {
    /// 1-based index. Matches the byte preceding the CF E0 marker in the
    /// payload's indexed sequence.
    pub block_index: u8,
    /// 16-char uppercase hex GUID, BE order, matching the convention used
    /// by `objects.guid`.
    pub block_guid: String,
}

/// Walk a payload for the indexed CF E0 effect-block sequence. Skips the
/// parent's self-reference (first CF E0 marker without an immediately
/// preceding 1-byte index that's both > 0 and < 0xCF).
///
/// The decoder is intentionally lenient: it accepts any contiguous-or-near-
/// contiguous run of `<idx> CF E0 00 <8-byte GUID>` records, with idx in
/// 0x01..0x40 (effect blocks are bounded; abilities like Saber Throw have
/// a few, complex multi-hit abilities have ~20 at most, so 0x40 caps the
/// false-positive risk on hash bytes that happen to look like an index).
pub fn extract_effect_block_refs(payload: &[u8]) -> Vec<EffectBlockRef> {
    let mut out = Vec::new();
    let mut last_idx: Option<u8> = None;
    let mut i = 0;
    // The first CF E0 marker in an ability/talent payload is the parent's
    // own content GUID (self-reference). Indexed effect-block refs start
    // AFTER that marker. Skip past it so the index-1 byte for the first
    // real effect block doesn't get conflated with the self-ref's preceding
    // byte (both can be 0x01).
    //
    // CF E0 marker shape per `docs/probes/dis-payload-format.md`:
    //   CF E0 00 + 6-byte GUID tail (full GUID = E000 + tail, 8 bytes BE)
    // Total marker = 9 bytes. The self-ref is the same 9-byte shape.
    let mut seen_self_ref = false;
    while i + 9 <= payload.len() {
        if payload[i] != 0xCF || payload[i + 1] != 0xE0 || payload[i + 2] != 0x00 {
            i += 1;
            continue;
        }
        if !seen_self_ref {
            seen_self_ref = true;
            i += 9;
            continue;
        }
        // CF E0 marker found AFTER the self-ref. The byte immediately
        // before it carries the 1-based index for indexed refs.
        // Per-record advance is +1 (not +9) because each indexed record is
        //   <idx> CF E0 00 <6-byte GUID tail>
        // so the next record's <idx> sits immediately after this one's
        // tail (1 byte before the next CF E0 marker).
        let prev = if i > 0 { Some(payload[i - 1]) } else { None };
        let expected_idx: u8 = last_idx.map(|n| n + 1).unwrap_or(1);
        if prev == Some(expected_idx) {
            let idx = expected_idx;
            let tail = &payload[i + 3..i + 9];
            let guid = format!("E000{}", hex::encode_upper(tail));
            out.push(EffectBlockRef {
                block_index: idx,
                block_guid: guid,
            });
            last_idx = Some(idx);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|s| u8::from_str_radix(s, 16).expect("hex"))
            .collect()
    }

    /// First ~160 bytes of Massacre's payload (verified against live spice).
    /// Contains: icon string, 4 inline float32 records, several CB/CC
    /// metadata markers, parent self-ref CF E0, then 4 indexed effect-block
    /// refs (indices 01..04).
    fn massacre_payload_prefix() -> Vec<u8> {
        hex_to_bytes(
            "00 00 00 00 00 40 13 CF 40 00 00 00 3D 2E 41 FD \
             06 09 6F 76 65 72 70 6F 77 65 72 01 04 00 00 80 \
             BF 20 04 00 00 40 40 02 04 CD CC CC 3E 02 04 00 \
             00 87 43 04 05 02 01 05 02 C8 C5 05 02 CB 51 3F \
             E4 54 03 01 CB 8F 00 59 0C 03 01 CB 7A 68 57 45 \
             03 01 CB B5 DE DA 55 02 C0 01 CC 07 60 79 A3 76 \
             01 CF E0 00 93 B2 44 62 08 65 CB 6F 3B F3 BC 07 \
             01 04 04 01 CF E0 00 03 46 25 CC E5 3B 02 CF E0 \
             00 02 46 25 CC E4 E8 03 CF E0 00 05 46 25 CC E9 \
             D1 04 CF E0 00 04 46 25 CC EB 86",
        )
    }

    #[test]
    fn extract_effect_block_refs_massacre_finds_4_indexed_refs() {
        let payload = massacre_payload_prefix();
        let refs = extract_effect_block_refs(&payload);
        assert_eq!(refs.len(), 4, "Massacre has 4 indexed effect-block refs");
        for (i, expected) in [
            (1u8, "E000034625CCE53B"),
            (2, "E000024625CCE4E8"),
            (3, "E000054625CCE9D1"),
            (4, "E000044625CCEB86"),
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(refs[i].block_index, expected.0);
            assert_eq!(refs[i].block_guid, expected.1);
        }
    }

    #[test]
    fn extract_effect_block_refs_skips_parent_self_ref() {
        let payload = massacre_payload_prefix();
        let refs = extract_effect_block_refs(&payload);
        // Massacre's own GUID E00093B244620865 must NOT appear in the
        // effect-block refs -- it's the parent self-ref, not an indexed
        // effect block.
        assert!(!refs.iter().any(|r| r.block_guid == "E00093B244620865"));
    }

    #[test]
    fn extract_effect_block_refs_handles_empty_payload() {
        assert!(extract_effect_block_refs(&[]).is_empty());
    }

    #[test]
    fn extract_effect_block_refs_handles_no_cf_e0_markers() {
        let payload = vec![0u8; 50];
        assert!(extract_effect_block_refs(&payload).is_empty());
    }
}
