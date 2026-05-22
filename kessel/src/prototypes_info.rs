//! Parser for `/resources/systemgenerated/prototypes.info` (PINF format).
//!
//! PINF is a metadata table mapping each `.node` PROT prototype's numeric ID
//! to a flag byte. The flag routes the .node file to its correct schema
//! converter (conversation vs creature vs stage vs player ability) without
//! parsing each file's header speculatively.
//!
//! Format (per sub-agent investigation, legion `019e4d74`):
//! - 11-byte header: `PINF` magic + version (`01 00 05 00`) + 3 unknown bytes
//! - Records: 10 bytes each = u64 BE numeric_id + u8 flag + 1 unknown byte
//! - Trailing end-of-stream marker
//!
//! Current archive: 723,690 records, 10,735 flag=1 (matches the cnv.* files
//! present in the .node corpus).

use anyhow::{bail, Result};
use std::collections::HashMap;

const PINF_MAGIC: &[u8; 4] = b"PINF";
const HEADER_LEN: usize = 11;
const RECORD_LEN: usize = 10;

/// One PINF record: numeric_id (matches the .node filename) and flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrototypeInfo {
    pub numeric_id: u64,
    pub flag: u8,
}

/// Parse the binary PINF file. Returns all records in file order.
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
        let id_bytes: [u8; 8] = chunk[0..8].try_into().unwrap();
        let numeric_id = u64::from_be_bytes(id_bytes);
        let flag = chunk[8];
        // chunk[9] is currently unknown -- carry along separately if/when meaning
        // is identified.
        records.push(PrototypeInfo { numeric_id, flag });
        i += RECORD_LEN;
    }

    Ok(records)
}

/// Build a numeric_id -> flag lookup map from a parsed record list.
#[allow(dead_code)]
pub fn build_id_to_flag_map(records: &[PrototypeInfo]) -> HashMap<u64, u8> {
    let mut map = HashMap::with_capacity(records.len());
    for record in records {
        map.insert(record.numeric_id, record.flag);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_record(id: u64, flag: u8) -> [u8; RECORD_LEN] {
        let mut out = [0u8; RECORD_LEN];
        out[..8].copy_from_slice(&id.to_be_bytes());
        out[8] = flag;
        out
    }

    fn build_pinf(records: &[(u64, u8)]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + records.len() * RECORD_LEN);
        bytes.extend_from_slice(PINF_MAGIC);
        bytes.extend_from_slice(&[0x01, 0x00, 0x05, 0x00]); // version
        bytes.extend_from_slice(&[0xAA, 0xAA, 0xAA]); // unknown header tail
        for (id, flag) in records {
            bytes.extend_from_slice(&build_record(*id, *flag));
        }
        bytes
    }

    #[test]
    fn parses_minimal_pinf() {
        let bytes = build_pinf(&[(0x12345678, 1)]);
        let records = parse(&bytes).expect("parse failed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].numeric_id, 0x12345678);
        assert_eq!(records[0].flag, 1);
    }

    #[test]
    fn parses_multiple_records() {
        let bytes = build_pinf(&[
            (14988260499516371179, 1),
            (14988260499516371180, 0),
            (14988260499516371181, 1),
        ]);
        let records = parse(&bytes).expect("parse failed");
        assert_eq!(records.len(), 3);
        let flag1 = records.iter().filter(|r| r.flag == 1).count();
        assert_eq!(flag1, 2);
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
    fn build_id_to_flag_map_dedupes() {
        let records = vec![
            PrototypeInfo {
                numeric_id: 1,
                flag: 0,
            },
            PrototypeInfo {
                numeric_id: 2,
                flag: 1,
            },
            PrototypeInfo {
                numeric_id: 1,
                flag: 1,
            }, // later entry wins
        ];
        let map = build_id_to_flag_map(&records);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&2), Some(&1));
    }
}
