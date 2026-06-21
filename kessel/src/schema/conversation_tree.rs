//! Decode a conversation NODE payload into its ordered dialogue lines (#285,
//! epic for #284-287). A `.node` prototype payload is a FLAT sequence of
//! standalone GOM objects; each dialogue-line object opens with the absolute
//! field marker `0x40000011_5CE87488` (the 9 bytes `CF 40 00 00 11 5C E8 74
//! 88`, consumed by `read_number`). That field's INT64 value's low 32 bits are
//! the line's string id; the immediately following field (`5CE87489`, a STRING)
//! is the `str.cnv` base FQN the line's text lives under.
//!
//! Proven by the #284 spike: marker byte order == dialogue order == ascending
//! `str.cnv` line id. All field ids drift across patches, so lines are located
//! by the marker SHAPE, not a schema lookup.

use crate::gom_reader::{GomValue, Reader};

/// The absolute field-id marker opening every dialogue-line node object.
const LINE_NODE_MARKER: [u8; 9] = [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
/// GOM type tag for a signed integer (the line-id field).
const TAG_INT64: u8 = 0x02;
/// GOM type tag for a string (the str.cnv ref field).
const TAG_STRING: u8 = 0x06;

/// One ordered dialogue line in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogueLine {
    /// 0-based position in the conversation (byte order == dialogue order).
    pub seq: u32,
    /// The line's string id -- the low 32 bits of the marker INT64; equals the
    /// `str.cnv` string's `id1` (its unique per-line id, not the shared `id2`).
    pub line_id: u32,
    /// The `str.cnv` base FQN the line's text lives under (e.g.
    /// `str.cnv.alliance.nar_shaddaa.misc.public_taxi`).
    pub line_ref: String,
    /// The `CF E0`-prefixed actor/ref GUIDs (16-char hex) in this node's byte
    /// range, in order -- the speaker/branch-ref candidates (#286/#287). The
    /// one resolving to a kind=Npc object is the speaker; one resolving to no
    /// Npc is the per-conversation player pseudo-actor.
    pub actor_guids: Vec<String>,
}

/// Decode every dialogue line from a conversation NODE payload, in byte order
/// (== dialogue order). Malformed nodes are skipped, not fatal.
pub fn decode_dialogue_lines(payload: &[u8]) -> Vec<DialogueLine> {
    // Collect line-node marker offsets first so each node's byte range
    // [pos, next) is known for the per-node actor-GUID scan.
    let mut positions = Vec::new();
    let mut w = 0usize;
    while w + LINE_NODE_MARKER.len() <= payload.len() {
        if payload[w..w + LINE_NODE_MARKER.len()] == LINE_NODE_MARKER {
            positions.push(w);
            w += LINE_NODE_MARKER.len();
        } else {
            w += 1;
        }
    }

    let mut out = Vec::new();
    let mut seq = 0u32;
    for (i, &pos) in positions.iter().enumerate() {
        let Some((line_id, line_ref)) = decode_node(payload, pos) else {
            continue;
        };
        let end = positions.get(i + 1).copied().unwrap_or(payload.len());
        out.push(DialogueLine {
            seq,
            line_id,
            line_ref,
            actor_guids: scan_e0_guids(payload, pos, end),
        });
        seq += 1;
    }
    out
}

/// Collect the `CF E0`-prefixed 8-byte GUIDs (as 16-char uppercase hex) within
/// `[start, end)` -- the node's speaker/branch-ref candidates.
fn scan_e0_guids(payload: &[u8], start: usize, end: usize) -> Vec<String> {
    let mut guids = Vec::new();
    let upper = end.min(payload.len()).saturating_sub(8);
    let mut w = start;
    while w < upper {
        if payload[w] == 0xCF && payload[w + 1] == 0xE0 {
            let g = u64::from_be_bytes(payload[w + 1..w + 9].try_into().unwrap());
            guids.push(format!("{g:016X}"));
        }
        w += 1;
    }
    guids
}

/// Decode the (line_id, str.cnv ref) of the node beginning at `pos` (the marker
/// offset). Returns `None` if the shape doesn't match (INT64 then STRING).
fn decode_node(payload: &[u8], pos: usize) -> Option<(u32, String)> {
    let mut r = Reader::new(payload, pos);
    let _fid = r.read_number().ok()?; // consume the 9-byte absolute marker
    if r.read_tag().ok()? != TAG_INT64 {
        return None;
    }
    let line_id = match r.read_value(TAG_INT64).ok()? {
        GomValue::I64(v) => v as u32, // low 32 bits == line_id (high byte is a constant)
        _ => return None,
    };
    let _delta = r.read_number().ok()?; // delta field-id to the str.cnv ref (5CE87489)
    if r.read_tag().ok()? != TAG_STRING {
        return None;
    }
    match r.read_value(TAG_STRING).ok()? {
        GomValue::Str(s) => Some((line_id, s)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_line_node() {
        // marker | INT64 tag | C9 03 E8 (=1000) | delta 01 | STRING tag | len 12 | ascii
        let mut p = vec![
            0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88, // marker
            TAG_INT64, 0xC9, 0x03, 0xE8, // line_id = 1000
            0x01, // delta to next field id
            TAG_STRING, 0x0C, // string len 12
        ];
        p.extend_from_slice(b"str.cnv.test");
        let lines = decode_dialogue_lines(&p);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].seq, 0);
        assert_eq!(lines[0].line_id, 1000);
        assert_eq!(lines[0].line_ref, "str.cnv.test");
    }

    #[test]
    fn line_id_is_low_32_bits() {
        // a 5-byte INT64 with a constant high byte 0x11 -> low32 is the id
        let mut p = vec![
            0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88, TAG_INT64, 0xCC, 0x11, 0x00,
            0x12, 0x46, 0xF2, // 0x11_0012_46F2 -> low32 = 0x1246F2 = 1197810
            0x01, TAG_STRING, 0x03,
        ];
        p.extend_from_slice(b"abc");
        let lines = decode_dialogue_lines(&p);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_id, 1_197_810);
    }

    #[test]
    fn no_marker_yields_nothing() {
        assert!(decode_dialogue_lines(b"no conversation markers here").is_empty());
    }

    #[test]
    fn captures_node_actor_guid() {
        let mut p = vec![
            0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88, TAG_INT64, 0xC9, 0x03, 0xE8,
            0x01, TAG_STRING, 0x03,
        ];
        p.extend_from_slice(b"abc");
        // a CF E0 actor GUID within the node's byte range
        p.extend_from_slice(&[0xCF, 0xE0, 0x00, 0x69, 0x4F, 0x37, 0x29, 0x0D, 0xFF]);
        let lines = decode_dialogue_lines(&p);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].actor_guids, vec!["E000694F37290DFF".to_string()]);
    }
}
