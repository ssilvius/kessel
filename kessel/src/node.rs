//! PROT (.node) prototype file parser.
//!
//! Files at `/resources/systemgenerated/prototypes/<numeric_id>.node` use the
//! PROT format:
//!
//! | offset | bytes | meaning                                          |
//! |--------|-------|--------------------------------------------------|
//! | 0      | 4     | `PROT` magic (50 52 4F 54)                       |
//! | 4      | 4     | version (`02 00 05 00` LE on v7.x archives)      |
//! | 8      | 8     | content GUID (u64 LE)                            |
//! | 16     | 4     | FQN length (u32 LE) -- includes trailing NUL     |
//! | 20     | N     | FQN UTF-8 bytes (may end in NUL)                 |
//! | 20+N   | ...   | binary GOM payload (same shape as PBUK payloads) |
//!
//! Empirically verified across 10,735 NODE entries in v7.x archives -- all
//! observed FQNs begin with `cnv.` (conversation prototypes). The format is
//! generic, so creature/stage/ability prototypes would parse identically if
//! they appear in future patches. The downstream GOM payload is handed off to
//! `kessel::pbuk` consumers (e.g. `extract_strings_from_payload`).
//!
//! Discovery reflection: legion `019dd668-d09c-7a20-b835-791e29f8e511`.

use crate::hash::HashDictionary;
use crate::myp::Archive;
use crate::prototypes_info::PrototypeInfo;
use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};

const PROT_MAGIC: [u8; 4] = [b'P', b'R', b'O', b'T'];
const HEADER_LEN: usize = 20;

/// One PROT-format node file's parsed contents.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NodeRecord {
    /// Numeric id matching the `.node` filename.
    pub numeric_id: u64,
    /// In-payload FQN (e.g. `cnv.<area>.<scene>`).
    pub fqn: String,
    /// PINF kind flag for downstream schema routing.
    pub kind_flag: u8,
    /// Raw GOM payload following the FQN -- ready for the pbuk decoder.
    pub payload: Vec<u8>,
    /// 16-char hex of the 8-byte content GUID at offset 8, matching the
    /// `objects` table format.
    pub template_guid: String,
}

/// Parse a single `.node` file's bytes into a `NodeRecord`.
#[allow(dead_code)]
pub fn parse(bytes: &[u8], numeric_id: u64, kind_flag: u8) -> Result<NodeRecord> {
    if bytes.len() < HEADER_LEN {
        bail!(
            "truncated PROT header: got {} bytes, need at least {}",
            bytes.len(),
            HEADER_LEN
        );
    }
    if bytes[..4] != PROT_MAGIC {
        bail!("invalid PROT magic: {:02X?}", &bytes[..4]);
    }

    let guid_bytes: [u8; 8] = bytes[8..16].try_into().expect("8..16 in 20-byte header");
    let template_guid = format!("{:016X}", u64::from_le_bytes(guid_bytes));

    let len_bytes: [u8; 4] = bytes[16..20].try_into().expect("16..20 in 20-byte header");
    let fqn_len = u32::from_le_bytes(len_bytes) as usize;
    let fqn_end = HEADER_LEN
        .checked_add(fqn_len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| {
            anyhow!(
                "invalid FQN length: {} (file size {})",
                fqn_len,
                bytes.len()
            )
        })?;

    let mut fqn_slice = &bytes[HEADER_LEN..fqn_end];
    while let Some(&0) = fqn_slice.last() {
        fqn_slice = &fqn_slice[..fqn_slice.len() - 1];
    }
    let fqn = std::str::from_utf8(fqn_slice)
        .map_err(|e| anyhow!("non-UTF8 FQN: {e}"))?
        .to_string();

    let payload = bytes[fqn_end..].to_vec();
    Ok(NodeRecord {
        numeric_id,
        fqn,
        kind_flag,
        payload,
        template_guid,
    })
}

/// Discover every `.node` file in the archive and parse each one.
///
/// `kind_flag` is looked up by content_guid against the PINF registry
/// (#180 model): each .node file's PROT header carries a content GUID at
/// bytes 8..16, and PINF maps that GUID to a routing flag. Records without
/// a PINF entry get flag `0`. Malformed PROT entries are skipped rather
/// than failing the whole walk -- the goal is "all parseable nodes" for
/// downstream extractors.
#[allow(dead_code)]
pub fn walk_archive_nodes(
    archive: &mut Archive,
    hash_dict: &HashDictionary,
    pinf: &[PrototypeInfo],
) -> Result<Vec<NodeRecord>> {
    let flag_by_guid: HashMap<String, u8> = pinf
        .iter()
        .map(|p| (p.content_guid.clone(), p.flag))
        .collect();
    let proto_hashes: HashSet<u64> = hash_dict
        .paths_matching("/resources/systemgenerated/prototypes/")
        .into_iter()
        .map(|(h, _)| h)
        .collect();
    let entries: Vec<_> = archive.entries()?.cloned().collect();

    let mut out = Vec::with_capacity(entries.len() / 16);
    for entry in entries {
        if !proto_hashes.contains(&entry.filename_hash) {
            continue;
        }
        let path = match hash_dict.get(entry.filename_hash) {
            Some(p) => p,
            None => continue,
        };
        let numeric_id = path
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".node"))
            .and_then(|s| s.parse::<u64>().ok());
        let Some(numeric_id) = numeric_id else {
            continue;
        };
        let bytes = match archive.read_entry(&entry) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Parse with placeholder flag 0; post-parse, look up flag by the
        // parsed content GUID and overwrite.
        if let Ok(mut rec) = parse(&bytes, numeric_id, 0) {
            rec.kind_flag = flag_by_guid.get(&rec.template_guid).copied().unwrap_or(0);
            out.push(rec);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_prot(fqn: &str, guid: u64, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_LEN + fqn.len() + 1 + payload.len());
        buf.extend_from_slice(&PROT_MAGIC);
        buf.extend_from_slice(&[0x02, 0x00, 0x05, 0x00]); // version
        buf.extend_from_slice(&guid.to_le_bytes());
        let len = (fqn.len() + 1) as u32; // include NUL terminator
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(fqn.as_bytes());
        buf.push(0);
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parses_minimal_cnv_prot() {
        let payload = vec![0xAA, 0xBB, 0xCC];
        let bytes = build_prot("cnv.test.scene_a", 0x1122_3344_5566_7788, &payload);
        let rec = parse(&bytes, 42, 1).expect("parse");
        assert_eq!(rec.numeric_id, 42);
        assert_eq!(rec.kind_flag, 1);
        assert_eq!(rec.fqn, "cnv.test.scene_a");
        assert_eq!(rec.template_guid, "1122334455667788");
        assert_eq!(rec.payload, payload);
    }

    #[test]
    fn rejects_invalid_magic() {
        let mut bytes = build_prot("cnv.x", 0, &[]);
        bytes[0] = b'X';
        let err = parse(&bytes, 0, 0).unwrap_err();
        assert!(format!("{err}").contains("invalid PROT magic"));
    }

    #[test]
    fn rejects_truncated_header() {
        let bytes = b"PROT\x02\x00\x05\x00\x00\x00";
        let err = parse(bytes, 0, 0).unwrap_err();
        assert!(format!("{err}").contains("truncated PROT header"));
    }

    #[test]
    fn rejects_fqn_length_past_eof() {
        let mut bytes = build_prot("cnv.short", 0, &[]);
        // overwrite length to point past file end
        bytes[16..20].copy_from_slice(&9_999u32.to_le_bytes());
        let err = parse(&bytes, 0, 0).unwrap_err();
        assert!(format!("{err}").contains("invalid FQN length"));
    }

    #[test]
    fn rejects_non_utf8_fqn() {
        let mut bytes = build_prot("cnv.placeholder", 0, &[]);
        // overwrite the first FQN byte with an invalid UTF-8 sequence start
        bytes[HEADER_LEN] = 0xFF;
        bytes[HEADER_LEN + 1] = 0xFE;
        let err = parse(&bytes, 0, 0).unwrap_err();
        assert!(format!("{err}").contains("non-UTF8"));
    }

    #[test]
    fn fqn_without_trailing_null_still_parses() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&PROT_MAGIC);
        buf.extend_from_slice(&[0x02, 0x00, 0x05, 0x00]);
        buf.extend_from_slice(&0u64.to_le_bytes());
        let fqn = "cnv.no_terminator";
        buf.extend_from_slice(&(fqn.len() as u32).to_le_bytes());
        buf.extend_from_slice(fqn.as_bytes());
        buf.extend_from_slice(&[0x11, 0x22]);
        let rec = parse(&buf, 7, 1).expect("parse");
        assert_eq!(rec.fqn, "cnv.no_terminator");
        assert_eq!(rec.payload, vec![0x11, 0x22]);
    }

    #[test]
    fn payload_is_slice_after_fqn() {
        let payload: Vec<u8> = (0..32).collect();
        let bytes = build_prot("cnv.payload_test", 1, &payload);
        let rec = parse(&bytes, 1, 0).expect("parse");
        assert_eq!(rec.payload, payload);
    }
}
