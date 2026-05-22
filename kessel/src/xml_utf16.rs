//! Shared UTF-16-LE BOM-detecting XML loader.
//!
//! Eight SWTOR archive file extensions use UTF-16-LE BOM-prefixed XML
//! (`FF FE 3C 00 ...`): `.epp`, `.svy`, `.tbl`, `.fxspec`, `.emt`, `.fxa`,
//! `.fxe`, plus UTF-8 XML siblings (`.mat`, `.tex`, `.xml`, `.rul`, etc).
//!
//! This module provides one BOM-detecting reader so each per-extension parser
//! doesn't reimplement UTF-16-to-UTF-8 transcoding plus quick_xml plumbing.
//!
//! BOM detection:
//!   - `FF FE`     -> UTF-16-LE
//!   - `FE FF`     -> UTF-16-BE
//!   - `EF BB BF`  -> UTF-8 (BOM stripped)
//!   - none        -> UTF-8 (assumed)

use anyhow::{bail, Result};

/// Detect a UTF-16 or UTF-8 BOM at the start of `bytes` and return an owned
/// UTF-8 String of the XML content. Strips BOM. Trims trailing nulls.
#[allow(dead_code)]
pub fn decode_xml_bom(bytes: &[u8]) -> Result<String> {
    if bytes.len() < 2 {
        bail!("input too short for BOM detection ({} bytes)", bytes.len());
    }

    let (kind, payload) = match (bytes[0], bytes[1]) {
        (0xFF, 0xFE) => (Encoding::Utf16Le, &bytes[2..]),
        (0xFE, 0xFF) => (Encoding::Utf16Be, &bytes[2..]),
        (0xEF, 0xBB) if bytes.len() >= 3 && bytes[2] == 0xBF => (Encoding::Utf8, &bytes[3..]),
        _ => (Encoding::Utf8, bytes),
    };

    let text = match kind {
        Encoding::Utf8 => std::str::from_utf8(payload)
            .map_err(|e| anyhow::anyhow!("invalid UTF-8: {e}"))?
            .to_string(),
        Encoding::Utf16Le => decode_utf16(payload, true)?,
        Encoding::Utf16Be => decode_utf16(payload, false)?,
    };

    Ok(text.trim_end_matches('\u{0}').to_string())
}

#[derive(Debug, Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        bail!("UTF-16 input has odd byte length: {}", bytes.len());
    }

    let u16_iter = bytes.chunks_exact(2).map(|chunk| {
        let pair = [chunk[0], chunk[1]];
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });

    let units: Vec<u16> = u16_iter.collect();
    char::decode_utf16(units)
        .collect::<std::result::Result<String, _>>()
        .map_err(|e| anyhow::anyhow!("invalid UTF-16 surrogate pair: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_utf16_le(text: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        for ch in text.chars() {
            let mut buf = [0u16; 2];
            let slice = ch.encode_utf16(&mut buf);
            for unit in slice {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        out
    }

    fn encode_utf16_be(text: &str) -> Vec<u8> {
        let mut out = vec![0xFE, 0xFF];
        for ch in text.chars() {
            let mut buf = [0u16; 2];
            let slice = ch.encode_utf16(&mut buf);
            for unit in slice {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        out
    }

    #[test]
    fn decodes_utf16_le_xml() {
        let bytes = encode_utf16_le("<Hello/>");
        let s = decode_xml_bom(&bytes).expect("decode");
        assert_eq!(s, "<Hello/>");
    }

    #[test]
    fn decodes_utf16_be_xml() {
        let bytes = encode_utf16_be("<World/>");
        let s = decode_xml_bom(&bytes).expect("decode");
        assert_eq!(s, "<World/>");
    }

    #[test]
    fn decodes_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<Hello/>");
        assert_eq!(decode_xml_bom(&bytes).expect("decode"), "<Hello/>");
    }

    #[test]
    fn decodes_utf8_without_bom() {
        let bytes = b"<Hello/>";
        assert_eq!(decode_xml_bom(bytes).expect("decode"), "<Hello/>");
    }

    #[test]
    fn rejects_too_short_input() {
        assert!(decode_xml_bom(b"").is_err());
        assert!(decode_xml_bom(b"X").is_err());
    }

    #[test]
    fn rejects_invalid_utf16_surrogate() {
        // BOM + an unpaired high surrogate
        let bytes = [0xFF, 0xFE, 0x00, 0xD8];
        assert!(decode_xml_bom(&bytes).is_err());
    }

    #[test]
    fn rejects_odd_length_utf16() {
        let bytes = [0xFF, 0xFE, 0x3C, 0x00, 0x00]; // BOM + 2 valid + 1 stray
        assert!(decode_xml_bom(&bytes).is_err());
    }

    #[test]
    fn trims_trailing_nulls() {
        let mut bytes = encode_utf16_le("<Hello/>");
        bytes.extend_from_slice(&[0x00, 0x00]); // extra null pair at end
        let s = decode_xml_bom(&bytes).expect("decode");
        assert!(!s.ends_with('\u{0}'), "trailing null not stripped: {s:?}");
    }

    #[test]
    fn handles_unicode_codepoints() {
        // Multi-byte UTF-16: U+1F600 (Grinning Face emoji) becomes a surrogate pair
        let bytes = encode_utf16_le("<Hi>\u{1F600}</Hi>");
        let s = decode_xml_bom(&bytes).expect("decode");
        assert_eq!(s, "<Hi>\u{1F600}</Hi>");
    }
}
