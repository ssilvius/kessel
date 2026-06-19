//! Decode an ability's description-token list -- the `<<N>>` positional tokens
//! in its description text.
//!
//! SWTOR ability descriptions interpolate values via `<<1>>`, `<<2>>` ...
//! tokens. Each token is declared in the ability payload's SpecParam list: an
//! ordered array (GOM field low32 `0x384b793a`) whose entries each carry a
//! token-type field (low32 `0x384b7939`) holding an `ablDescriptionTokenType`
//! enum value -- Rank / Damage / Healing / Duration / Bindpoint / Absorption /
//! Stat. `<<N>>` indexes entry `N-1`.
//!
//! This names WHAT each token represents (the description structure). The
//! numeric VALUE for Damage/Healing tokens is level/stat-scaled and computed at
//! runtime (a coefficient in the effect graph, not a static literal); Duration
//! and Rank tokens are fixed but live in the referenced effect. So this decoder
//! resolves token *meaning*, the durable static signal.
//!
//! Validated against the in-game oracle: abl.agent.evasion `<<1>>` = Duration;
//! abl.agent.corrosive_dart `<<1>>` = Damage, `<<2>>` = Duration;
//! abl.agent.diagnostic_scan `<<1>>` = Healing.
//!
//! GOM field ids drift across patches; the two ids below are current as of the
//! 7.9 schema and matched directly (same convention as the granted-ability
//! field 0x2d7b8786 elsewhere). If a future patch yields zero tokens corpus
//! wide, re-derive them with `kessel-discovery/examples/probe_specparams`.

use crate::gom_reader::{read_object_fields, GomValue};
use crate::gom_schema;

/// SpecParam list field (the ordered `<<N>>` token array).
const SPECPARAM_LIST_FIELD: u32 = 0x384b_793a;
/// Per-entry token-type field (an `ablDescriptionTokenType` enum value).
const TOKEN_TYPE_FIELD: u32 = 0x384b_7939;
/// The client.gom enum naming the token types.
const TOKEN_TYPE_ENUM: &str = "ablDescriptionTokenType";

/// One `<<N>>` description token: its 0-based index (`<<1>>` -> 0) and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescToken {
    pub index: u32,
    /// Lowercased token type with the `ablDescriptionTokenType` prefix stripped
    /// (`damage`, `healing`, `duration`, `rank`, `absorption`, `bindpoint`,
    /// `stat`), or `unknown_<n>` for an enum value the embedded dictionary
    /// doesn't name.
    pub token_type: String,
}

/// Decode the `<<N>>` description-token types from an ability GOM payload.
/// Returns an empty vec for payloads with no SpecParam list (most non-ability
/// objects, and abilities whose descriptions take no parameters).
pub fn decode_desc_tokens(payload: &[u8]) -> Vec<DescToken> {
    let Ok(obj) = read_object_fields(payload) else {
        return Vec::new();
    };
    let Some(list) = obj
        .embedded_field(SPECPARAM_LIST_FIELD)
        .and_then(GomValue::as_list)
    else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for (i, entry) in list.iter().enumerate() {
        let Some(type_idx) = entry.embedded_field(TOKEN_TYPE_FIELD).and_then(GomValue::as_i64)
        else {
            continue;
        };
        let token_type = gom_schema::enum_member(TOKEN_TYPE_ENUM, type_idx)
            .map(strip_token_prefix)
            .unwrap_or_else(|| format!("unknown_{type_idx}"));
        out.push(DescToken {
            index: i as u32,
            token_type,
        });
    }
    out
}

/// `ablDescriptionTokenTypeDuration` -> `duration`.
fn strip_token_prefix(member: &str) -> String {
    member
        .strip_prefix("ablDescriptionTokenType")
        .unwrap_or(member)
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_lowercases() {
        assert_eq!(strip_token_prefix("ablDescriptionTokenTypeDuration"), "duration");
        assert_eq!(strip_token_prefix("ablDescriptionTokenTypeDamage"), "damage");
        assert_eq!(strip_token_prefix("weird"), "weird");
    }
}
