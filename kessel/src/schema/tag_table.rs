//! Decoder for `tagTablePrototype` and tag-hash lookup for ability/talent
//! payloads. Issue #174.
//!
//! Wire format (verified against live extract 2026-05-25):
//!
//! The payload is a flat array of records. Each record is one of two shapes:
//!
//!   CE-form (44 legacy records):
//!     CE <7-byte tag hash> <1-byte name length> <ASCII tag.* name>
//!
//!   CF-form (6706 records):
//!     CF <8-byte tag hash> <1-byte name length> <ASCII tag.* name>
//!
//! Total dictionary: 6750 entries. All names start with `tag.abl.*`.
//!
//! Linkage: an ability or talent "has" a tag when the tag's hash bytes appear
//! anywhere in the parent's payload. Critically, the linkage data often lives
//! on the NON-CANONICAL variant of an ability (the longer payload kessel
//! deduplicates), so the cross-reference must scan rows for every FQN, not
//! just `is_canonical = 1`.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRecord {
    /// 14 or 16 hex chars (uppercase), matching the marker form.
    pub tag_hash: String,
    pub tag_fqn: String,
    /// `"CE"` or `"CF"`.
    pub hash_marker: &'static str,
}

/// Decode the full tagTablePrototype payload. Records that fail the
/// `<length><tag.*>` shape check (e.g. trailing padding) are skipped.
pub fn decode_tag_table(payload: &[u8]) -> Vec<TagRecord> {
    let mut out = Vec::new();
    let needle = b"tag.";
    let mut idx = 0;
    while idx + needle.len() <= payload.len() {
        if &payload[idx..idx + needle.len()] != needle {
            idx += 1;
            continue;
        }
        let mut end = idx;
        while end < payload.len()
            && payload[end] >= 0x20
            && payload[end] < 0x7F
            && payload[end] != b' '
        {
            end += 1;
        }
        let len = end - idx;
        if idx < 10 || payload[idx - 1] as usize != len {
            idx = end.max(idx + 1);
            continue;
        }
        let name = match std::str::from_utf8(&payload[idx..end]) {
            Ok(s) => s.to_string(),
            Err(_) => {
                idx = end;
                continue;
            }
        };
        let len_pos = idx - 1;
        // CE form: marker at len_pos - 8, hash at len_pos - 7 .. len_pos.
        // CF form: marker at len_pos - 9, hash at len_pos - 8 .. len_pos.
        if len_pos >= 8 && payload[len_pos - 8] == 0xCE {
            let hash = hex::encode_upper(&payload[len_pos - 7..len_pos]);
            out.push(TagRecord {
                tag_hash: hash,
                tag_fqn: name,
                hash_marker: "CE",
            });
        } else if len_pos >= 9 && payload[len_pos - 9] == 0xCF {
            let hash = hex::encode_upper(&payload[len_pos - 8..len_pos]);
            out.push(TagRecord {
                tag_hash: hash,
                tag_fqn: name,
                hash_marker: "CF",
            });
        }
        idx = end;
    }
    out
}

/// Hash-lookup tables for fast cross-reference against ability/talent payloads.
pub struct TagIndex {
    /// 7-byte hash → tag FQN (44 CE records).
    pub ce_by_hash: HashMap<[u8; 7], String>,
    /// 8-byte hash → tag FQN (6706 CF records).
    pub cf_by_hash: HashMap<[u8; 8], String>,
}

impl TagIndex {
    pub fn build(records: &[TagRecord]) -> Self {
        let mut ce = HashMap::new();
        let mut cf = HashMap::new();
        for r in records {
            let raw = hex::decode(&r.tag_hash).unwrap_or_default();
            match r.hash_marker {
                "CE" if raw.len() == 7 => {
                    let mut h = [0u8; 7];
                    h.copy_from_slice(&raw);
                    ce.insert(h, r.tag_fqn.clone());
                }
                "CF" if raw.len() == 8 => {
                    let mut h = [0u8; 8];
                    h.copy_from_slice(&raw);
                    cf.insert(h, r.tag_fqn.clone());
                }
                _ => {}
            }
        }
        TagIndex {
            ce_by_hash: ce,
            cf_by_hash: cf,
        }
    }

    /// Scan a payload for every tag-hash reference. Returns unique tag FQNs.
    pub fn scan_payload(&self, payload: &[u8]) -> Vec<String> {
        use std::collections::BTreeSet;
        let mut hits: BTreeSet<String> = BTreeSet::new();
        let mut i = 0;
        while i + 7 <= payload.len() {
            let mut h = [0u8; 7];
            h.copy_from_slice(&payload[i..i + 7]);
            if let Some(name) = self.ce_by_hash.get(&h) {
                hits.insert(name.clone());
            }
            i += 1;
        }
        i = 0;
        while i + 8 <= payload.len() {
            let mut h = [0u8; 8];
            h.copy_from_slice(&payload[i..i + 8]);
            if let Some(name) = self.cf_by_hash.get(&h) {
                hits.insert(name.clone());
            }
            i += 1;
        }
        hits.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|s| u8::from_str_radix(s, 16).expect("hex"))
            .collect()
    }

    /// Two real records from tagTablePrototype: one CE-form (44-record legacy
    /// set) and one CF-form (6706-record main set). Verified against live
    /// extract 2026-05-25 via probe_tag_record_gaps.
    fn two_record_fixture() -> Vec<u8> {
        // CE form: CE 07 C5 57 22 AE FE 5A + 0x48 (len 72) + 72-char name
        // CF form: CF 02 04 8F 8D CC 0D 18 97 + 0x21 (len 33) + 33-char name
        //   (using "tag.abl.exp.uprisings.placeholder" as a synthetic 33-char fixture)
        let ce_name = "tag.abl.qtr.flashpoint.rishi.flashpoint_2.mob.boss.boss_1.spy.in_stealth";
        assert_eq!(ce_name.len(), 72);
        let cf_name = "tag.abl.exp.uprisings.placeholder";
        assert_eq!(cf_name.len(), 33);
        let mut bytes = Vec::new();
        // Pad header so first record's marker has room behind it.
        bytes.extend_from_slice(&hex_to_bytes("00 00 00 01 01 02 03 04 05 06 07 08"));
        // CE record
        bytes.extend_from_slice(&hex_to_bytes("CE 07 C5 57 22 AE FE 5A 48"));
        bytes.extend_from_slice(ce_name.as_bytes());
        // CF record
        bytes.extend_from_slice(&hex_to_bytes("CF 02 04 8F 8D CC 0D 18 97 21"));
        bytes.extend_from_slice(cf_name.as_bytes());
        bytes
    }

    #[test]
    fn decode_tag_table_recovers_both_marker_forms() {
        let bytes = two_record_fixture();
        let recs = decode_tag_table(&bytes);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].hash_marker, "CE");
        assert_eq!(recs[0].tag_hash, "07C55722AEFE5A");
        assert_eq!(
            recs[0].tag_fqn,
            "tag.abl.qtr.flashpoint.rishi.flashpoint_2.mob.boss.boss_1.spy.in_stealth"
        );
        assert_eq!(recs[1].hash_marker, "CF");
        assert_eq!(recs[1].tag_hash, "02048F8DCC0D1897");
        assert_eq!(recs[1].tag_fqn, "tag.abl.exp.uprisings.placeholder");
    }

    #[test]
    fn tag_index_finds_hashes_in_payload() {
        let recs = decode_tag_table(&two_record_fixture());
        let idx = TagIndex::build(&recs);
        // Synthetic ability payload: header bytes + CE hash + filler + CF hash
        let mut payload = vec![0u8; 16];
        payload.extend_from_slice(&hex_to_bytes("07 C5 57 22 AE FE 5A"));
        payload.extend_from_slice(&[0xFF; 8]);
        payload.extend_from_slice(&hex_to_bytes("02 04 8F 8D CC 0D 18 97"));
        let hits = idx.scan_payload(&payload);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|t| t.contains("in_stealth")));
        assert!(hits.iter().any(|t| t.contains("placeholder")));
    }

    #[test]
    fn decode_handles_empty_payload() {
        assert!(decode_tag_table(&[]).is_empty());
    }

    #[test]
    fn scan_payload_handles_empty() {
        let idx = TagIndex::build(&[]);
        assert!(idx.scan_payload(&[]).is_empty());
    }
}
