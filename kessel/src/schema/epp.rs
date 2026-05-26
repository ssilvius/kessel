//! `.epp` (Appearance Spec) UTF-16-LE XML decoder. Issue #183.
//!
//! Wire format: UTF-16-LE BOM + XML rooted at
//! `<Appearance fqn="..." GUID="...">`. Each appearance has one or more
//! `<SubAppearance>` blocks. Each SubAppearance carries an
//! `<AppearanceActionList>` of `<AppearanceAction type="...">` elements.
//! The action types observed in v7.x include `PlayAnimAppFunctionType`,
//! `PlayFXAppFunctionType`, `PopWeaponInAppFunctionType`, etc.
//!
//! `<AppearanceAction type="PlayFXAppFunctionType">` blocks contain a
//! `<fxSpecString refType="weak_fxSpec">PATH</fxSpecString>` element that
//! names the `.fxspec` resource by path-relative key (e.g.
//! `abilities/sith_warrior/sw_massacre_sword_glow`).
//!
//! This decoder extracts the fqn, the list of action types, and the list
//! of fxSpec references. Per-action typed-field decoding (caster bones,
//! priority classes, animation note triggers) is left for downstream
//! consumers operating on the raw_xml field.

use anyhow::{anyhow, Result};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceSpec {
    /// FQN from the root `<Appearance fqn="...">` attribute.
    pub fqn: String,
    /// Distinct `<AppearanceAction type="...">` values in document order.
    pub appearance_actions: Vec<String>,
    /// Distinct `<fxSpecString>PATH</fxSpecString>` values in document order.
    pub fx_spec_refs: Vec<String>,
    /// Full decoded UTF-8 XML text. Preserved for downstream debug + future
    /// per-action typed-field consumers.
    pub raw_xml: String,
}

fn root_fqn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<Appearance\b[^>]*\bfqn="([^"]+)""#).unwrap())
}

fn action_type_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<AppearanceAction\b[^>]*\btype="([^"]+)""#).unwrap())
}

fn fxspec_string_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<fxSpecString\b[^>]*>([^<]+)</fxSpecString>").unwrap())
}

/// Decode a UTF-16-LE EPP XML file into a structured AppearanceSpec.
pub fn decode_epp(bytes: &[u8]) -> Result<AppearanceSpec> {
    let raw_xml = crate::xml_utf16::decode_xml_bom(bytes)?;
    let fqn = root_fqn_re()
        .captures(&raw_xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| anyhow!("EPP root <Appearance fqn=...> not found"))?;

    let mut appearance_actions = Vec::new();
    for cap in action_type_re().captures_iter(&raw_xml) {
        let v = cap.get(1).unwrap().as_str().to_string();
        if !appearance_actions.contains(&v) {
            appearance_actions.push(v);
        }
    }

    let mut fx_spec_refs = Vec::new();
    for cap in fxspec_string_re().captures_iter(&raw_xml) {
        let v = cap.get(1).unwrap().as_str().trim().to_string();
        if !v.is_empty() && !fx_spec_refs.contains(&v) {
            fx_spec_refs.push(v);
        }
    }

    Ok(AppearanceSpec {
        fqn,
        appearance_actions,
        fx_spec_refs,
        raw_xml,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_epp(xml: &str) -> Vec<u8> {
        let mut bytes = vec![0xFFu8, 0xFE];
        for c in xml.encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decode_epp_extracts_fqn_actions_and_fxspecs() {
        let xml = r#"<Appearance fqn="epp.sith_warrior.massacre.cast_instant" GUID="2028117916909568">
   <SubAppearanceList>
      <SubAppearance>
         <AppearanceActionList>
            <AppearanceAction type="PlayAnimAppFunctionType">
               <casterAnim>cb_2saber_sp2_attack_right_12</casterAnim>
            </AppearanceAction>
            <AppearanceAction type="PopWeaponInAppFunctionType">
               <whichActor>caster</whichActor>
            </AppearanceAction>
            <AppearanceAction type="PlayFXAppFunctionType">
               <fxSpecString refType="weak_fxSpec">abilities/sith_warrior/sw_massacre_sword_glow</fxSpecString>
            </AppearanceAction>
            <AppearanceAction type="PlayFXAppFunctionType">
               <fxSpecString refType="weak_fxSpec">audio_ability_general_dummy</fxSpecString>
            </AppearanceAction>
         </AppearanceActionList>
      </SubAppearance>
   </SubAppearanceList>
</Appearance>"#;
        let bytes = synth_epp(xml);
        let spec = decode_epp(&bytes).unwrap();
        assert_eq!(spec.fqn, "epp.sith_warrior.massacre.cast_instant");
        assert_eq!(
            spec.appearance_actions,
            vec![
                "PlayAnimAppFunctionType",
                "PopWeaponInAppFunctionType",
                "PlayFXAppFunctionType"
            ]
        );
        assert_eq!(
            spec.fx_spec_refs,
            vec![
                "abilities/sith_warrior/sw_massacre_sword_glow",
                "audio_ability_general_dummy"
            ]
        );
    }

    #[test]
    fn decode_epp_errors_on_missing_root() {
        let bytes = synth_epp("<NotAppearance/>");
        assert!(decode_epp(&bytes).is_err());
    }

    #[test]
    fn decode_epp_handles_empty_action_list() {
        let xml = r#"<Appearance fqn="epp.x">
   <SubAppearanceList>
      <SubAppearance>
         <AppearanceActionList />
      </SubAppearance>
   </SubAppearanceList>
</Appearance>"#;
        let bytes = synth_epp(xml);
        let spec = decode_epp(&bytes).unwrap();
        assert_eq!(spec.fqn, "epp.x");
        assert!(spec.appearance_actions.is_empty());
        assert!(spec.fx_spec_refs.is_empty());
    }
}
