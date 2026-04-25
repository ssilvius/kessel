//! PBUK/DBLB Parser
//!
//! Current SWTOR format (2024+):
//!
//! PBUK Container:
//!   - bytes 0-3: "PBUK" magic
//!   - bytes 4-5: chunk count (u16) - typically 2
//!   - bytes 6-7: unknown (u16)
//!   - bytes 8-11: offset to first DBLB (always 12)
//!   - byte 12+: DBLB wrapper followed by object DBLB
//!
//! DBLB Wrapper (16 bytes, at offset 12):
//!   - bytes 0-3: "DBLB" magic
//!   - bytes 4-7: version (u32, typically 2)
//!   - bytes 8-11: padding (zeros)
//!   - bytes 12-15: total DBLB size
//!
//! Object DBLB (at offset 28):
//!   - bytes 0-3: "DBLB" magic
//!   - bytes 4-7: version (u32)
//!   - bytes 8-11: first object size (u32)
//!   - bytes 12-15: padding
//!   - byte 16+: objects
//!
//! Object format:
//!   - 42-byte header (contains GUIDs, offsets)
//!   - FQN string (null-terminated)
//!   - padding to align
//!   - ZSTD-compressed payload (trim last 8 bytes)
//!   - 8-byte footer (next object link)
//!
//! The ZSTD payload contains binary GOM data with length-prefixed strings.

use anyhow::{bail, Context, Result};

const PBUK_MAGIC: [u8; 4] = [b'P', b'B', b'U', b'K'];
const DBLB_MAGIC: [u8; 4] = [b'D', b'B', b'L', b'B'];
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

// Object size bounds based on observed SWTOR data
const MIN_OBJECT_SIZE: usize = 50;
const MAX_OBJECT_SIZE: usize = 50000;
// Safety limit to prevent infinite loops
const MAX_OBJECTS_PER_DBLB: usize = 10000;

/// Check if data starts with PBUK magic
pub fn is_pbuk(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == PBUK_MAGIC
}

/// Check if data starts with DBLB magic
pub fn is_dblb(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == DBLB_MAGIC
}

/// A parsed GOM object from DBLB
#[derive(Debug, Clone)]
pub struct GomObject {
    /// Fully Qualified Name (e.g., "itm.gen.lots.weapon...")
    pub fqn: String,
    /// Raw header bytes (42 bytes, contains GUIDs)
    pub header: Vec<u8>,
    /// Decompressed GOM payload (binary format)
    pub payload: Vec<u8>,
}

/// Extract ASCII strings from a raw GOM payload.
///
/// GOM uses at least two encodings empirically:
///
/// 1. Canonical: `0x06 <length_byte> <ASCII>` -- common in spawn/encounter payloads.
/// 2. With prefix: `0x06 <flag bytes> <length_byte> <ASCII>` -- common in quest payloads,
///    e.g., `0x06 01 01 01 <len> <ASCII>`. The intermediate bytes appear to be
///    flag/array-count metadata between the marker and the actual string.
///
/// To catch both, we combine two heuristics: prefer the canonical 0x06 marker
/// pattern, and fall back to a plain `<length_byte> <ASCII>` scan everywhere
/// else. The fallback recovers strings whose marker is followed by
/// intermediate bytes -- it lands on the actual length byte since the
/// intermediates fail the canonical check.
///
/// This is a heuristic, not a full msgpack-style decode. If a future format
/// change introduces ambiguity, the right fix is to decode the type-tag
/// stream properly rather than to refine the heuristics.
pub fn extract_strings_from_payload(payload: &[u8]) -> Vec<String> {
    let mut strings = Vec::new();
    let mut i = 0;

    while i + 2 <= payload.len() {
        // Canonical: 0x06 <len> <ascii>
        if payload[i] == 0x06 {
            let len = payload[i + 1] as usize;
            let start = i + 2;
            if (2..200).contains(&len) && start + len <= payload.len() {
                let candidate = &payload[start..start + len];
                if candidate.iter().all(|&b| (32..127).contains(&b)) {
                    if let Ok(s) = std::str::from_utf8(candidate) {
                        strings.push(s.to_string());
                        i = start + len;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }

        // Fallback: <len> <ascii> directly. Catches strings preceded by
        // intermediate bytes after the 0x06 marker (e.g., `06 01 01 01 a7 ...`)
        // by landing on the actual length byte once the canonical scan has
        // walked past the intermediates.
        let len = payload[i] as usize;
        let start = i + 1;
        if (2..200).contains(&len) && start + len <= payload.len() {
            let candidate = &payload[start..start + len];
            if candidate.iter().all(|&b| (32..127).contains(&b)) {
                if let Ok(s) = std::str::from_utf8(candidate) {
                    strings.push(s.to_string());
                    i = start + len;
                    continue;
                }
            }
        }
        i += 1;
    }

    strings
}

impl GomObject {
    /// Try to extract strings from the binary payload
    pub fn extract_strings(&self) -> Vec<String> {
        extract_strings_from_payload(&self.payload)
    }
}

/// Parse a PBUK container, extracting all GOM objects
pub fn parse(data: &[u8]) -> Result<Vec<GomObject>> {
    if !is_pbuk(data) {
        bail!("Not a PBUK file");
    }

    // PBUK structure:
    // - 12 byte header
    // - 16 byte DBLB wrapper (at offset 12)
    // - Object DBLB (at offset 28)

    if data.len() < 44 {
        bail!("PBUK too small");
    }

    // Verify DBLB wrapper at offset 12
    if data[12..16] != DBLB_MAGIC {
        bail!("No DBLB wrapper at offset 12");
    }

    // Verify object DBLB at offset 28
    if data[28..32] != DBLB_MAGIC {
        bail!("No object DBLB at offset 28");
    }

    // Parse object DBLB
    let objects_dblb = &data[28..];
    parse_object_dblb(objects_dblb)
}

/// Try to extract next object size from the 8-byte footer.
/// SWTOR stores the size at varying positions in the footer.
fn extract_next_size_from_footer(footer: &[u8]) -> Option<usize> {
    if footer.len() != 8 {
        return None;
    }

    // Find first non-zero byte and read size from there
    let first_nonzero = footer.iter().position(|&b| b != 0)?;

    // Try reading as u16 LE
    if first_nonzero + 2 <= 8 {
        let val = u16::from_le_bytes([footer[first_nonzero], footer[first_nonzero + 1]]) as usize;
        if val > MIN_OBJECT_SIZE && val < MAX_OBJECT_SIZE {
            return Some(val);
        }
    }

    // Single byte fallback
    let val = footer[first_nonzero] as usize;
    if val > MIN_OBJECT_SIZE {
        Some(val)
    } else {
        None
    }
}

/// Parse the object DBLB block using a hybrid approach:
/// 1. Use footer chain for fast parsing (no ZSTD probing)
/// 2. Fall back to ZSTD probing when footer chain breaks
fn parse_object_dblb(data: &[u8]) -> Result<Vec<GomObject>> {
    if !is_dblb(data) {
        bail!("Not a DBLB block");
    }

    let mut objects = Vec::new();

    // DBLB header: 16 bytes, first object size at bytes 8-11
    let first_obj_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;

    tracing::debug!("Object DBLB: first object size = {}", first_obj_size);

    let mut offset = 16;
    let mut obj_size = first_obj_size;
    let mut using_footer_chain = true;

    while offset + MIN_OBJECT_SIZE < data.len() && objects.len() < MAX_OBJECTS_PER_DBLB {
        if using_footer_chain && obj_size > MIN_OBJECT_SIZE && offset + obj_size <= data.len() {
            // Fast path: parse using known object size from footer chain
            let obj_data = &data[offset..offset + obj_size];

            // Validate: check FQN looks valid
            let fqn_valid =
                obj_data.len() > 46 && obj_data[42..46].iter().all(|&b| (32..127).contains(&b));

            if fqn_valid {
                // Get footer for next iteration BEFORE trying to parse
                let footer = &obj_data[obj_data.len() - 8..];
                let next_size = extract_next_size_from_footer(footer);

                // Try to parse - may fail for non-ZSTD objects (which is ok)
                if let Ok(obj) = parse_object(obj_data) {
                    objects.push(obj);
                }
                // Note: We continue the chain even if parse_object fails
                // Some objects (ipp.*, stg.*, etc) don't have ZSTD payloads

                // Move to next object
                let next_unaligned = offset + obj_size;
                offset = if !next_unaligned.is_multiple_of(8) {
                    next_unaligned + (8 - next_unaligned % 8)
                } else {
                    next_unaligned
                };

                match next_size {
                    Some(s) if s > MIN_OBJECT_SIZE && s < MAX_OBJECT_SIZE => {
                        obj_size = s;
                        continue;
                    }
                    _ => {
                        // Footer chain broken, switch to scanning
                        using_footer_chain = false;
                    }
                }
            } else {
                using_footer_chain = false;
            }
        }

        // Slow path: scan for objects by FQN pattern
        let fqn_pos = offset + 42;
        if fqn_pos + 4 >= data.len() {
            break;
        }

        let potential_fqn = &data[fqn_pos..data.len().min(fqn_pos + 4)];
        let has_fqn = potential_fqn.iter().all(|&b| (32..127).contains(&b));

        if !has_fqn {
            offset += 8;
            continue;
        }

        // Find end of FQN
        let mut fqn_end = fqn_pos;
        while fqn_end < data.len() && data[fqn_end] != 0 {
            fqn_end += 1;
        }

        // Find ZSTD magic
        let mut zstd_pos = None;
        for i in fqn_end..data.len().min(fqn_end + 10) {
            if data.len() > i + 4 && data[i..i + 4] == ZSTD_MAGIC {
                zstd_pos = Some(i);
                break;
            }
        }

        if let Some(zstd_start) = zstd_pos {
            // Use ZSTD's frame size detection - O(1) instead of probing
            let zstd_data = &data[zstd_start..];
            if let Ok(frame_size) = zstd_safe::find_frame_compressed_size(zstd_data) {
                if frame_size > 0 && zstd_start + frame_size <= data.len() {
                    if let Ok(decoded) = zstd::decode_all(&zstd_data[..frame_size]) {
                        let obj_end = zstd_start + frame_size + 8;
                        let fqn = String::from_utf8_lossy(&data[fqn_pos..fqn_end]).to_string();
                        let header =
                            data[offset..offset.saturating_add(42).min(data.len())].to_vec();

                        objects.push(GomObject {
                            fqn,
                            header,
                            payload: decoded,
                        });

                        offset = obj_end;
                        if offset % 8 != 0 {
                            offset += 8 - (offset % 8);
                        }
                    } else {
                        offset = fqn_end + 8;
                    }
                } else {
                    offset = fqn_end + 8;
                }
            } else {
                offset = fqn_end + 8;
            }
        } else {
            offset = fqn_end + 8;
        }
    }

    tracing::debug!("Parsed {} GOM objects", objects.len());
    Ok(objects)
}

/// Parse a single object given its exact bytes
fn parse_object(data: &[u8]) -> Result<GomObject> {
    if data.len() < 50 {
        bail!("Object too small");
    }

    // Header is 42 bytes
    let header = data[0..42].to_vec();

    // Find FQN starting at offset 42
    let fqn_start = 42;
    let mut fqn_end = fqn_start;
    while fqn_end < data.len() && data[fqn_end] != 0 {
        fqn_end += 1;
    }

    let fqn = String::from_utf8_lossy(&data[fqn_start..fqn_end]).to_string();

    // Find ZSTD magic after FQN null
    let mut zstd_pos = None;
    for i in fqn_end..data.len().min(fqn_end + 10) {
        if data.len() > i + 4 && data[i..i + 4] == ZSTD_MAGIC {
            zstd_pos = Some(i);
            break;
        }
    }

    let zstd_start = zstd_pos.context("No ZSTD magic found")?;

    // ZSTD payload ends 8 bytes before object end
    if data.len() < 8 {
        bail!("Object too small for footer");
    }
    let payload_end = data.len() - 8;

    if payload_end <= zstd_start {
        bail!("Invalid payload bounds");
    }

    let zstd_payload = &data[zstd_start..payload_end];
    let payload = zstd::decode_all(zstd_payload).context("Failed to decompress ZSTD payload")?;

    Ok(GomObject {
        fqn,
        header,
        payload,
    })
}

/// Parse a standalone DBLB file (not wrapped in PBUK)
pub fn parse_dblb_direct(data: &[u8]) -> Result<Vec<GomObject>> {
    // For direct DBLB, skip the wrapper and go straight to object parsing
    if !is_dblb(data) {
        bail!("Not a DBLB block");
    }

    // Check if this is a wrapper DBLB (has another DBLB at offset 16)
    if data.len() > 20 && data[16..20] == DBLB_MAGIC {
        // Skip wrapper
        parse_object_dblb(&data[16..])
    } else {
        parse_object_dblb(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode strings the way GOM does: `0x06 <length> <ASCII>`.
    fn pack(strings: &[&str]) -> Vec<u8> {
        let mut buf = Vec::new();
        for s in strings {
            buf.push(0x06);
            buf.push(s.len() as u8);
            buf.extend_from_slice(s.as_bytes());
        }
        buf
    }

    #[test]
    fn extract_strings_finds_marker_prefixed_ascii() {
        let payload = pack(&["npc.korriban.foo", "enc.korriban.bar"]);
        let strings = extract_strings_from_payload(&payload);
        assert!(strings.contains(&"npc.korriban.foo".to_string()));
        assert!(strings.contains(&"enc.korriban.bar".to_string()));
    }

    #[test]
    fn extract_strings_skips_short_runs() {
        // Length 1 is below the minimum (2) and should be skipped, not yielded.
        let payload = pack(&["x", "npc.real"]);
        let strings = extract_strings_from_payload(&payload);
        assert!(!strings.iter().any(|s| s == "x"));
        assert!(strings.iter().any(|s| s == "npc.real"));
    }

    #[test]
    fn extract_strings_handles_a_prefix_encounter_refs() {
        // Quest payloads reference encounters with the `a:` type marker.
        // The free function returns the raw string; callers strip the prefix.
        let payload = pack(&["a:enc.korriban.tomb"]);
        let strings = extract_strings_from_payload(&payload);
        assert!(strings.iter().any(|s| s == "a:enc.korriban.tomb"));
    }

    #[test]
    fn extract_strings_uses_fallback_when_marker_is_absent() {
        // When the canonical 0x06 marker is preceded by intermediate bytes
        // before the length byte, the fallback heuristic recovers the string
        // by reading directly at the length byte.
        let mut payload = vec![0x06, 0x01, 0x01, 0x01, 0x41]; // marker + intermediates + length=65
        payload.extend_from_slice(
            b"spn.location.korriban.mob.tomb_2_marka_ragnos.mob02.standard_m_01",
        );
        let strings = extract_strings_from_payload(&payload);
        assert!(
            strings
                .iter()
                .any(|s| s.starts_with("spn.location.korriban")),
            "fallback must recover string after intermediate bytes, got {:?}",
            strings
        );
    }

    #[test]
    fn extract_strings_finds_spn_triple_in_quest_payload() {
        // Quest payloads embed NPC references inside semicolon-delimited triples:
        // `spn.X;npc.Y;<numeric_id>`. The whole triple is one string in GOM,
        // so the scanner must yield it intact for downstream parsing.
        let triple = b"spn.location.korriban.class.sith_warrior.judge_and_executioner.jailer_knash;npc.location.korriban.class.sith_warrior.judge_and_executioner.jailer_knash;291310451818496";
        let mut payload = vec![0x06, triple.len() as u8];
        payload.extend_from_slice(triple);
        let strings = extract_strings_from_payload(&payload);
        assert_eq!(strings.len(), 1);
        assert!(strings[0].starts_with("spn."));
        assert!(strings[0].contains(";npc."));
    }

    #[test]
    fn extract_strings_finds_full_fqn_when_marker_is_a_substring_byte() {
        // The length byte 0x41 ('A') is itself printable ASCII. The previous
        // heuristic incorrectly emitted "Aspn.l" because it read the marker
        // 0x06 as length=6. With the marker scan, the full 65-char FQN is
        // emitted instead.
        let mut payload = vec![0x06, 0x41];
        payload.extend_from_slice(
            b"spn.location.korriban.mob.tomb_2_marka_ragnos.mob02.standard_m_01",
        );
        let strings = extract_strings_from_payload(&payload);
        assert_eq!(
            strings,
            vec!["spn.location.korriban.mob.tomb_2_marka_ragnos.mob02.standard_m_01".to_string()]
        );
    }
}
