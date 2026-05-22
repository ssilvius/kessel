//! `.epp` Appearance Prototype parser.
//!
//! Files at `/resources/gamedata/epp/<numeric_id>.epp` are UTF-16-LE BOM
//! XML documents declaring per-object visual appearance data: FX bindings,
//! material slots, bone references, etc. 20,515 entries in v7.x archives
//! per sub-agent E (legion `019e4d74`).
//!
//! This parser handles the BOM via `kessel::xml_utf16::decode_xml_bom` and
//! uses `quick-xml`'s event reader to avoid allocating a full DOM. Unknown
//! elements and attributes are ignored so future game patches add fields
//! without breaking extraction.

use crate::hash::HashDictionary;
use crate::myp::Archive;
use crate::xml_utf16::decode_xml_bom;
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;
use std::collections::HashSet;

/// One parsed `.epp` appearance prototype.
#[derive(Debug, Clone, Serialize, Default)]
#[allow(dead_code)]
pub struct Appearance {
    pub fqn: String,
    pub guid: String,
    pub asset_version: u32,
    pub creation_time_stamp: String,
    pub fx_actions: Vec<FxAction>,
}

/// One `<AppearanceAction>` child describing an FX trigger.
#[derive(Debug, Clone, Serialize, Default)]
#[allow(dead_code)]
pub struct FxAction {
    pub kind: String,
    pub fx_spec_string: Option<String>,
    pub caster_target_type: Option<String>,
    pub target_bone: Option<String>,
    pub is_looping: Option<bool>,
    pub fx_priority: Option<String>,
    pub fx_channel: Option<String>,
}

/// Parse an `.epp` file's bytes into an `Appearance` record.
#[allow(dead_code)]
pub fn parse(bytes: &[u8]) -> Result<Appearance> {
    let xml = decode_xml_bom(bytes).context("epp BOM decode")?;
    let mut reader = Reader::from_str(&xml);
    reader.trim_text(true);

    let mut appearance = Appearance::default();
    let mut buf = Vec::new();
    let mut current_fx: Option<FxAction> = None;
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name_owned = e.name().as_ref().to_vec();
                let name = std::str::from_utf8(&name_owned).unwrap_or("");
                match name {
                    "Appearance" => {
                        saw_root = true;
                        for attr in e.attributes().with_checks(false).flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                "fqn" => appearance.fqn = val,
                                "GUID" => appearance.guid = normalize_guid(&val),
                                "assetVersion" => {
                                    appearance.asset_version = val.parse().unwrap_or(0)
                                }
                                "CreationTimeStamp" => appearance.creation_time_stamp = val,
                                _ => {}
                            }
                        }
                    }
                    "AppearanceAction" => {
                        let mut action = FxAction::default();
                        for attr in e.attributes().with_checks(false).flatten() {
                            if attr.key.as_ref() == b"type" {
                                action.kind = attr.unescape_value().unwrap_or_default().to_string();
                            }
                        }
                        current_fx = Some(action);
                    }
                    other => {
                        if let Some(action) = current_fx.as_mut() {
                            let value = collect_text_attr(&e, &mut reader)?;
                            assign_fx_field(action, other, &value);
                        }
                    }
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"AppearanceAction" => {
                if let Some(action) = current_fx.take() {
                    appearance.fx_actions.push(action);
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(anyhow!("epp xml parse: {err}")),
            _ => {}
        }
        buf.clear();
    }

    if !saw_root {
        bail!("missing <Appearance> root element");
    }
    if appearance.fqn.is_empty() {
        bail!("missing fqn attribute on <Appearance>");
    }
    Ok(appearance)
}

/// Walk every `.epp` entry in the archive and parse each.
///
/// Per-file errors are silently skipped -- the goal is "all parseable
/// appearances" for downstream extractors. Callers that want failure
/// detail should call `parse` directly on the bytes.
#[allow(dead_code)]
pub fn walk_archive_appearances(
    archive: &mut Archive,
    hash_dict: &HashDictionary,
) -> Result<Vec<Appearance>> {
    let epp_hashes: HashSet<u64> = hash_dict
        .paths_matching("/resources/gamedata/epp/")
        .into_iter()
        .filter(|(_, path)| path.ends_with(".epp"))
        .map(|(h, _)| h)
        .collect();
    let entries: Vec<_> = archive.entries()?.cloned().collect();

    let mut out = Vec::with_capacity(epp_hashes.len());
    for entry in entries {
        if !epp_hashes.contains(&entry.filename_hash) {
            continue;
        }
        let bytes = match archive.read_entry(&entry) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(rec) = parse(&bytes) {
            out.push(rec);
        }
    }
    Ok(out)
}

fn normalize_guid(raw: &str) -> String {
    let stripped: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let no_prefix = stripped.strip_prefix("0x").unwrap_or(&stripped);
    no_prefix.to_uppercase()
}

fn collect_text_attr<R: std::io::BufRead>(
    e: &quick_xml::events::BytesStart<'_>,
    reader: &mut Reader<R>,
) -> Result<String> {
    for attr in e.attributes().with_checks(false).flatten() {
        if attr.key.as_ref() == b"value" {
            return Ok(attr.unescape_value().unwrap_or_default().to_string());
        }
    }
    let mut inner_buf = Vec::new();
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut inner_buf) {
            Ok(Event::Text(t)) => {
                text.push_str(&t.unescape().unwrap_or_default());
            }
            Ok(Event::End(end)) if end.name() == e.name() => break,
            Ok(Event::Eof) => break,
            Err(err) => return Err(anyhow!("epp inner text: {err}")),
            _ => {}
        }
        inner_buf.clear();
    }
    Ok(text)
}

fn assign_fx_field(action: &mut FxAction, name: &str, value: &str) {
    let v = value.to_string();
    match name {
        "FxSpecString" => action.fx_spec_string = Some(v),
        "CasterTargetType" => action.caster_target_type = Some(v),
        "TargetBone" => action.target_bone = Some(v),
        "IsLooping" => action.is_looping = Some(matches!(v.as_str(), "true" | "1" | "True")),
        "FxPriority" => action.fx_priority = Some(v),
        "FxChannel" => action.fx_channel = Some(v),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_epp(xml: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        for c in xml.encode_utf16() {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_minimal_appearance() {
        let xml = r#"<?xml version="1.0" encoding="utf-16"?>
<Appearance fqn="epp.test.foo" GUID="0xDEADBEEFCAFEBABE" assetVersion="3" CreationTimeStamp="2026-01-01"/>"#;
        let bytes = build_epp(xml);
        let app = parse(&bytes).expect("parse");
        assert_eq!(app.fqn, "epp.test.foo");
        assert_eq!(app.guid, "DEADBEEFCAFEBABE");
        assert_eq!(app.asset_version, 3);
        assert_eq!(app.creation_time_stamp, "2026-01-01");
        assert!(app.fx_actions.is_empty());
    }

    #[test]
    fn parses_appearance_with_fx_actions() {
        let xml = r#"<?xml version="1.0" encoding="utf-16"?>
<Appearance fqn="epp.test.bar" GUID="01" assetVersion="1" CreationTimeStamp="now">
  <AppearanceAction type="PlayFXAppFunctionType">
    <FxSpecString value="fx.spec.example"/>
    <CasterTargetType value="SINGLENODE"/>
    <TargetBone value="head"/>
    <IsLooping value="true"/>
    <FxPriority value="HIGH"/>
    <FxChannel value="VISUAL"/>
  </AppearanceAction>
</Appearance>"#;
        let bytes = build_epp(xml);
        let app = parse(&bytes).expect("parse");
        assert_eq!(app.fx_actions.len(), 1);
        let fx = &app.fx_actions[0];
        assert_eq!(fx.kind, "PlayFXAppFunctionType");
        assert_eq!(fx.fx_spec_string.as_deref(), Some("fx.spec.example"));
        assert_eq!(fx.caster_target_type.as_deref(), Some("SINGLENODE"));
        assert_eq!(fx.target_bone.as_deref(), Some("head"));
        assert_eq!(fx.is_looping, Some(true));
        assert_eq!(fx.fx_priority.as_deref(), Some("HIGH"));
        assert_eq!(fx.fx_channel.as_deref(), Some("VISUAL"));
    }

    #[test]
    fn ignores_unknown_attributes_and_elements() {
        let xml = r#"<?xml version="1.0" encoding="utf-16"?>
<Appearance fqn="epp.unknown" GUID="00" assetVersion="1" CreationTimeStamp="0" unknown_attr="x">
  <UnknownChild ignore="me"/>
  <AppearanceAction type="PlayFXAppFunctionType">
    <UnknownField value="z"/>
  </AppearanceAction>
</Appearance>"#;
        let bytes = build_epp(xml);
        let app = parse(&bytes).expect("parse");
        assert_eq!(app.fx_actions.len(), 1);
        assert_eq!(app.fx_actions[0].kind, "PlayFXAppFunctionType");
    }

    #[test]
    fn rejects_missing_root() {
        let xml = r#"<NotAppearance/>"#;
        let bytes = build_epp(xml);
        let err = parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("missing <Appearance>"));
    }

    #[test]
    fn rejects_missing_fqn() {
        let xml = r#"<Appearance GUID="00" assetVersion="1" CreationTimeStamp="0"/>"#;
        let bytes = build_epp(xml);
        let err = parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("missing fqn"));
    }

    #[test]
    fn normalizes_guid_to_uppercase_no_prefix() {
        assert_eq!(normalize_guid("0xdeadbeef"), "DEADBEEF");
        assert_eq!(normalize_guid(" 01ab "), "01AB");
        assert_eq!(normalize_guid("ABCDEF"), "ABCDEF");
    }
}
