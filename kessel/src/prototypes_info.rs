//! Parser for `/resources/systemgenerated/prototypes.info` (PINF format).
//!
//! PINF is a content-GUID registry with one record per known prototype. The
//! per-record `flag` is the routing key for the prototype's payload class.
//!
//! Wire format (verified against live v7.x archive 2026-05-26 -- see
//! `docs/probes/pinf-format.md`):
//!
//! ```text
//! | offset | bytes | meaning                                          |
//! |--------|-------|--------------------------------------------------|
//! | 0      | 4     | `PINF` magic (50 49 4E 46)                       |
//! | 4      | 4     | version (`01 00 05 00` LE on v7.x)               |
//! | 8      | 4     | unknown header tail                              |
//! | 12     | N×10  | records, 10 bytes each                           |
//! ```
//!
//! Per record (10 bytes):
//!
//! ```text
//! | offset | bytes | meaning                                          |
//! |--------|-------|--------------------------------------------------|
//! | 0      | 1     | `CF` constant marker                             |
//! | 1      | 8     | content GUID (BE, `E000`-prefixed)               |
//! | 9      | 1     | flag (1 = cnv NODE prototype, 2/3 = TBD)         |
//! ```
//!
//! Live archive: 723,690 records. Flag=1 hits exactly 10,735 (matches the
//! cnv NODE corpus count). Flag=2 and flag=3 sub-categorize the remaining
//! prototypes; their precise semantics is a follow-on investigation.
//!
//! History: a prior parser interpreted bytes 0..8 of each record as a u64
//! numeric_id and byte 8 as flag, with HEADER_LEN=11. That produced the
//! "flag bytes uniformly distributed across 0x00–0xFF" symptom that issue
//! #180 was opened to investigate. Both bugs are fixed here.

use anyhow::{bail, Result};
use std::collections::HashMap;

const PINF_MAGIC: &[u8; 4] = b"PINF";
const HEADER_LEN: usize = 12;
const RECORD_LEN: usize = 10;
const RECORD_MARKER: u8 = 0xCF;

/// One PINF record: content GUID + routing flag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PrototypeInfo {
    /// 16-char uppercase hex content GUID, BE order, matching `objects.guid`.
    pub content_guid: String,
    /// Routing flag. Empirically only three values in v7.x: 1 (cnv NODE
    /// prototype), 2 (typed NODE prototype), 3 (everything else).
    pub flag: u8,
}

/// Parse the binary PINF file. Returns all records in file order. Records
/// with a leading byte other than `CF` are skipped (defensive: PINF is
/// expected to be uniformly shaped, but we don't bail on a single bad row).
#[allow(dead_code)]
pub fn parse(bytes: &[u8]) -> Result<Vec<PrototypeInfo>> {
    if bytes.len() < HEADER_LEN {
        bail!("PINF too short for header: {} bytes", bytes.len());
    }
    if &bytes[..4] != PINF_MAGIC {
        bail!(
            "invalid PINF magic: {:02X?} (expected {:02X?})",
            &bytes[..4],
            PINF_MAGIC
        );
    }

    let payload = &bytes[HEADER_LEN..];
    let usable_len = (payload.len() / RECORD_LEN) * RECORD_LEN;
    let mut records = Vec::with_capacity(usable_len / RECORD_LEN);

    let mut i = 0;
    while i + RECORD_LEN <= usable_len {
        let chunk = &payload[i..i + RECORD_LEN];
        if chunk[0] == RECORD_MARKER {
            let content_guid = hex::encode_upper(&chunk[1..9]);
            let flag = chunk[9];
            records.push(PrototypeInfo { content_guid, flag });
        }
        i += RECORD_LEN;
    }

    Ok(records)
}

/// Build a content_guid -> flag lookup map from a parsed record list.
#[allow(dead_code)]
pub fn build_guid_to_flag_map(records: &[PrototypeInfo]) -> HashMap<String, u8> {
    let mut map = HashMap::with_capacity(records.len());
    for record in records {
        map.insert(record.content_guid.clone(), record.flag);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_record(guid_hex: &str, flag: u8) -> [u8; RECORD_LEN] {
        let bytes = hex::decode(guid_hex).expect("8-byte GUID hex");
        assert_eq!(bytes.len(), 8);
        let mut out = [0u8; RECORD_LEN];
        out[0] = RECORD_MARKER;
        out[1..9].copy_from_slice(&bytes);
        out[9] = flag;
        out
    }

    fn build_pinf(records: &[(&str, u8)]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + records.len() * RECORD_LEN);
        bytes.extend_from_slice(PINF_MAGIC);
        bytes.extend_from_slice(&[0x01, 0x00, 0x05, 0x00]); // version
        bytes.extend_from_slice(&[0xCA, 0x0B, 0x0A, 0xEA]); // unknown header tail
        for (guid, flag) in records {
            bytes.extend_from_slice(&build_record(guid, *flag));
        }
        bytes
    }

    #[test]
    fn parses_minimal_pinf() {
        let bytes = build_pinf(&[("E00000000208C02E", 3)]);
        let records = parse(&bytes).expect("parse failed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].content_guid, "E00000000208C02E");
        assert_eq!(records[0].flag, 3);
    }

    #[test]
    fn parses_real_first_record_fixture() {
        // First two records from the live v7.x PINF, verified against the
        // archive 2026-05-26.
        let bytes = build_pinf(&[
            ("E00000000208C02E", 3),
            ("E000000023ADD96F", 3),
            ("E000000056C4ED0B", 1),
        ]);
        let records = parse(&bytes).expect("parse failed");
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].content_guid, "E00000000208C02E");
        assert_eq!(records[1].content_guid, "E000000023ADD96F");
        assert_eq!(records[2].content_guid, "E000000056C4ED0B");
        assert_eq!(records[2].flag, 1, "cnv NODE prototype flag");
    }

    #[test]
    fn rejects_invalid_magic() {
        let mut bytes = vec![b'N', b'O', b'P', b'E'];
        bytes.extend_from_slice(&[0u8; HEADER_LEN]);
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_too_short() {
        let bytes = b"PINF\x01\x00";
        assert!(parse(bytes).is_err());
    }

    #[test]
    fn empty_records_section_ok() {
        let bytes = build_pinf(&[]);
        let records = parse(&bytes).expect("parse failed");
        assert!(records.is_empty());
    }

    #[test]
    fn build_guid_to_flag_map_dedupes() {
        let records = vec![
            PrototypeInfo {
                content_guid: "E00000000208C02E".into(),
                flag: 3,
            },
            PrototypeInfo {
                content_guid: "E000000023ADD96F".into(),
                flag: 2,
            },
            PrototypeInfo {
                content_guid: "E00000000208C02E".into(),
                flag: 1, // later entry wins
            },
        ];
        let map = build_guid_to_flag_map(&records);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("E00000000208C02E"), Some(&1));
        assert_eq!(map.get("E000000023ADD96F"), Some(&2));
    }

    #[test]
    fn skips_record_with_bad_marker() {
        // Construct a 2-record PINF where the second record's leading byte
        // is not CF -- it should be skipped, leaving 1 record.
        let mut bytes = build_pinf(&[("E00000000208C02E", 3)]);
        // Append a bogus record (10 bytes starting with 00).
        bytes.extend_from_slice(&[0x00, 0xE0, 0x00, 0x00, 0x00, 0x23, 0xAD, 0xD9, 0x6F, 0x03]);
        let records = parse(&bytes).expect("parse failed");
        assert_eq!(records.len(), 1);
    }
}
