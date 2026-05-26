//! `.fxspec` (FX Spec / nodeWClasses) UTF-16-LE XML decoder. Issue #183.
//!
//! Wire format: UTF-16-LE without BOM. The XML root is `<nodeWClasses>`
//! with a `<classes>` block listing class names and a `<marshalData>`
//! block carrying per-node-instance field data. The class names are the
//! routing/type information; the marshalData is per-node attribute soup.
//!
//! This decoder extracts the FQN (derived from the source file path by
//! the caller, since the XML itself doesn't carry one), the list of class
//! names, and preserves the full raw_xml for downstream consumers that
//! want per-node detail.

use anyhow::{anyhow, Result};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxSpec {
    /// FQN derived from the source path (e.g.
    /// `art.fx.fxspec.abilities.sith_warrior.sw_massacre_sword_glow`).
    /// The XML body does not embed the FQN; the populator computes it from
    /// the resource path before calling `decode_fxspec`.
    pub fqn: String,
    /// Distinct `<class>NAME</class>` values inside the `<classes>` block.
    pub node_classes: Vec<String>,
    /// Full decoded UTF-8 XML text. Preserved for downstream typed-field
    /// consumers (per-node marshalData parsing is out of scope here).
    pub raw_xml: String,
}

fn class_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<class>([^<]+)</class>").unwrap())
}

/// Decode a UTF-16-LE FX-spec XML file. `fqn` is supplied by the caller
/// because the XML body itself doesn't embed one.
pub fn decode_fxspec(bytes: &[u8], fqn: String) -> Result<FxSpec> {
    let raw_xml = decode_utf16_le_or_bom(bytes)?;
    let mut node_classes = Vec::new();
    for cap in class_re().captures_iter(&raw_xml) {
        let v = cap.get(1).unwrap().as_str().trim().to_string();
        if !v.is_empty() && !node_classes.contains(&v) {
            node_classes.push(v);
        }
    }
    Ok(FxSpec {
        fqn,
        node_classes,
        raw_xml,
    })
}

/// Decode bytes as UTF-16-LE, accepting an optional BOM. .fxspec files
/// in v7.x archives ship WITHOUT a BOM but are still UTF-16-LE; .epp files
/// ship WITH the `FF FE` BOM. This function handles both shapes plus a
/// UTF-8 fallback for safety.
fn decode_utf16_le_or_bom(bytes: &[u8]) -> Result<String> {
    if bytes.len() < 2 {
        return Err(anyhow!("fxspec too short: {} bytes", bytes.len()));
    }
    let (kind, payload) = match (bytes[0], bytes[1]) {
        (0xFF, 0xFE) => ("utf16le", &bytes[2..]),
        (0xFE, 0xFF) => ("utf16be", &bytes[2..]),
        // No BOM but the second byte is 0x00 -- characteristic of UTF-16-LE
        // ASCII content (which all SWTOR fxspec files are).
        (_, 0x00) => ("utf16le", bytes),
        _ => ("utf8", bytes),
    };
    let s = match kind {
        "utf16le" | "utf16be" => {
            if payload.len() % 2 != 0 {
                return Err(anyhow!("UTF-16 payload has odd byte count"));
            }
            let units: Vec<u16> = payload
                .chunks_exact(2)
                .map(|c| {
                    if kind == "utf16le" {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            String::from_utf16(&units).map_err(|e| anyhow!("invalid UTF-16: {e}"))?
        }
        _ => std::str::from_utf8(payload)
            .map_err(|e| anyhow!("invalid UTF-8: {e}"))?
            .to_string(),
    };
    Ok(s.trim_end_matches('\u{0}').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_fxspec_le(xml: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        for c in xml.encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decode_fxspec_extracts_class_names() {
        let xml = r"<nodeWClasses><classes><class>_FxSpec</class><class>_FxSound</class></classes><marshalData/></nodeWClasses>";
        let bytes = synth_fxspec_le(xml);
        let spec = decode_fxspec(&bytes, "art.fx.fxspec.test".to_string()).unwrap();
        assert_eq!(spec.fqn, "art.fx.fxspec.test");
        assert_eq!(spec.node_classes, vec!["_FxSpec", "_FxSound"]);
    }

    #[test]
    fn decode_fxspec_handles_bom_prefix() {
        let xml = r"<nodeWClasses><classes><class>_FxSpec</class></classes></nodeWClasses>";
        let mut bytes = vec![0xFFu8, 0xFE];
        bytes.extend(synth_fxspec_le(xml));
        let spec = decode_fxspec(&bytes, "art.fx.fxspec.bom_variant".to_string()).unwrap();
        assert_eq!(spec.node_classes, vec!["_FxSpec"]);
    }

    #[test]
    fn decode_fxspec_dedupes_class_names() {
        let xml = r"<nodeWClasses><classes><class>_FxSpec</class><class>_FxSpec</class></classes></nodeWClasses>";
        let bytes = synth_fxspec_le(xml);
        let spec = decode_fxspec(&bytes, "x".into()).unwrap();
        assert_eq!(spec.node_classes, vec!["_FxSpec"]);
    }

    #[test]
    fn decode_fxspec_errors_on_truncated_payload() {
        let bytes = [0xFFu8];
        assert!(decode_fxspec(&bytes, "x".into()).is_err());
    }
}
