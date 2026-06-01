//! Shared `Database`-agnostic helpers used across the db domain modules.
//!
//! Pure byte/string/FQN utilities + the generic item-object walker. Kept free
//! of `Database` references so this stays a leaf module the domain files depend
//! on (never the reverse).

use crate::db::DISCIPLINE_COMBAT_STYLE_MAP;
use anyhow::Result;
use rusqlite::{Connection, Transaction};

/// Count non-overlapping occurrences of a byte pattern in a payload.
/// Used by singleton extraction to record cheap shape hints (CF E0 marker
/// count, CF 40 marker count) without committing to a full decoder pass.
pub(crate) fn count_byte_pattern(payload: &[u8], pattern: &[u8]) -> usize {
    if pattern.is_empty() || payload.len() < pattern.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + pattern.len() <= payload.len() {
        if &payload[i..i + pattern.len()] == pattern {
            count += 1;
            i += pattern.len();
        } else {
            i += 1;
        }
    }
    count
}

/// Pull every ASCII run of length >= `min_len` from a payload, returning
/// the runs as `String`s in payload order. Used by typed-detail populators
/// to find well-known string tokens (pkg.aggro.*, role labels, etc.) without
/// needing per-property byte-layout decode.
pub(crate) fn extract_ascii_strings(payload: &[u8], min_len: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < payload.len() {
        if (0x20..0x7F).contains(&payload[i]) {
            let start = i;
            while i < payload.len() && (0x20..0x7F).contains(&payload[i]) {
                i += 1;
            }
            if i - start >= min_len {
                if let Ok(s) = std::str::from_utf8(&payload[start..i]) {
                    out.push(s.to_string());
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Find every `cf 40 00 00` GOM record-marker offset in a payload. Advances
/// past each 4-byte hit so overlapping matches are not double-counted.
pub(crate) fn cf40_marker_positions(payload: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut i = 0;
    while i + 4 <= payload.len() {
        if payload[i] == 0xCF
            && payload[i + 1] == 0x40
            && payload[i + 2] == 0
            && payload[i + 3] == 0
        {
            positions.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }
    positions
}

/// Find the byte sequence `prefix` in `record`, then read the length-prefixed
/// ASCII string that follows it (`<prefix><len: u8><len bytes>`). Returns None
/// if the prefix is absent, the length runs past the slice, or the bytes are
/// not valid UTF-8.
pub(crate) fn find_length_prefixed_string(record: &[u8], prefix: &[u8]) -> Option<String> {
    if prefix.is_empty() || record.len() < prefix.len() {
        return None;
    }
    let limit = record.len() - prefix.len();
    for i in 0..=limit {
        if &record[i..i + prefix.len()] == prefix {
            let len_pos = i + prefix.len();
            let len = *record.get(len_pos)? as usize;
            let start = len_pos + 1;
            let end = start + len;
            if end <= record.len() {
                return std::str::from_utf8(&record[start..end])
                    .ok()
                    .map(String::from);
            }
            return None;
        }
    }
    None
}

/// Extract every typed float (`04` tag immediately followed by an f32 LE) from
/// a GOM record body, in order. Skips past each consumed float so bytes inside
/// a float value are not re-scanned as a new tag.
pub(crate) fn typed_floats_in(body: &[u8]) -> Vec<f32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 <= body.len() {
        if body[i] == 0x04 {
            let bytes: [u8; 4] = body[i + 1..i + 5].try_into().unwrap();
            out.push(f32::from_le_bytes(bytes));
            i += 5;
        } else {
            i += 1;
        }
    }
    out
}

pub(crate) fn is_fqn_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

/// Walk every canonical `itm.*` object: decode its payload with the
/// typed-value GOM reader and hand the decoded object to `f` along with its
/// FQN and game_id. Centralizes the select + decode loop shared by the
/// per-item populators (`item_stats`, `item_granted_abilities`, ...).
///
/// Rows with no payload are skipped silently (storage-level absence). A
/// *structural* decode failure -- bytes present and valid base64 but the GOM
/// walker hit an unmodeled tag -- is the signal that the archive has drifted
/// (a new item shape or a schema-version bump); these are counted and a
/// summary is logged via `tracing::warn!`, never silently dropped. The reader
/// decodes every item payload in the current archive, so a non-zero count is a
/// real "investigate this" event, not routine.
pub(crate) fn for_each_item_object(
    tx: &Transaction,
    mut f: impl FnMut(&str, Option<&str>, &crate::gom_reader::GomValue) -> Result<()>,
) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let mut select = tx.prepare(
        "SELECT fqn, game_id, json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'itm.%' AND is_canonical = 1",
    )?;
    let rows: Vec<(String, Option<String>, Option<String>)> = select
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(select);
    let mut decode_failures = 0u64;
    for (fqn, game_id, payload_b64) in rows {
        let Some(payload_b64) = payload_b64 else {
            continue;
        };
        let Ok(payload) = BASE64.decode(&payload_b64) else {
            continue;
        };
        let obj = match crate::gom_reader::read_object_fields(&payload) {
            Ok(obj) => obj,
            Err(e) => {
                // Structural decode failure: surface the first few, then count.
                if decode_failures < 5 {
                    tracing::warn!("item payload decode failed for {fqn}: {e}");
                }
                decode_failures += 1;
                continue;
            }
        };
        f(&fqn, game_id.as_deref(), &obj)?;
    }
    if decode_failures > 0 {
        tracing::warn!(
            "for_each_item_object: {decode_failures} item payload(s) failed to decode \
             (archive may have drifted -- a new item shape or schema bump)"
        );
    }
    Ok(())
}

/// Pull `(fqn, payload_b64)` tuples for every object of `kind`. Used by
/// the populate_* passes that need to walk binary payloads.
pub(crate) fn fetch_fqn_payloads(conn: &Connection, kind: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare("SELECT fqn, json_extract(json, '$.payload_b64') FROM objects WHERE kind = ?1 AND is_canonical = 1")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([kind], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Parse the SPN-triple format that appears in quest payloads:
///
/// ```text
/// spn.<faction.planet.path>;<target_fqn>;<numeric_id>
/// ```
///
/// Returns all three parts, or None if the string is not a well-formed
/// SPN triple. Caller decides whether to keep based on `target_fqn`'s
/// prefix (npc/plc/etc.).
pub(crate) fn parse_spn_triple(s: &str) -> Option<(String, String, u64)> {
    if !s.starts_with("spn.") {
        return None;
    }
    let mut parts = s.splitn(3, ';');
    let spn_fqn = parts.next()?;
    let target_fqn = parts.next()?;
    let numeric_str = parts.next()?;
    let runtime_id = numeric_str.parse::<u64>().ok()?;
    Some((spn_fqn.to_string(), target_fqn.to_string(), runtime_id))
}

/// Convenience: extract just the npc.* target from an SPN triple, or None
/// if the triple is malformed or its target is not an NPC.
pub(crate) fn npc_from_spn_triple(s: &str) -> Option<String> {
    let (_spn, target, _id) = parse_spn_triple(s)?;
    if target.starts_with("npc.") {
        Some(target)
    } else {
        None
    }
}

/// Decode tier level from FQN segment like "tier2" → 23, "tier3" → 39, etc.
/// Maps SWTOR's tier numbering to actual level requirements.
/// Convert a snake_case FQN segment to a title-cased display name.
/// e.g. `mag_bolt` -> `Mag Bolt`, `fueled_corruption` -> `Fueled Corruption`.
/// Used by `backfill_missing_string_ids` to derive a candidate display name
/// when the GOM payload lacks a string-table marker.
pub(crate) fn title_case_from_snake(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Origin -> 2 combat styles. Used to fan per-origin shared/utility pools
/// (abl.<origin>.<name>, abl.<origin>.skill.utility.*, abl.<origin>.skill.mods.*,
/// tal.<origin>.skill.utility.*) into combat_style_shared_abilities and
/// class_utility_talents.
pub(crate) fn origin_combat_styles(origin: &str) -> &'static [&'static str] {
    match origin {
        "sith_warrior" => &["juggernaut", "marauder"],
        "sith_inquisitor" => &["assassin", "sorcerer"],
        "bounty_hunter" => &["powertech", "mercenary"],
        "agent" => &["operative", "sniper"],
        "jedi_knight" => &["guardian", "sentinel"],
        "jedi_consular" => &["shadow", "force_wizard"],
        "trooper" => &["specialist", "commando"],
        "smuggler" => &["scoundrel", "gunslinger"],
        _ => &[],
    }
}

pub(crate) fn combat_style_for(origin: &str, discipline: &str) -> Option<&'static str> {
    DISCIPLINE_COMBAT_STYLE_MAP
        .iter()
        .find(|(o, d, _)| *o == origin && *d == discipline)
        .map(|(_, _, cs)| *cs)
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Extract the destination planet from a quest transit string.
///
/// Only fires on the explicit travel verbs `travel_to_{dest}` / `traveled_to_{dest}`
/// (e.g. `qm_traveled_to_yavin_4`, `qm_travel_to_korriban`, `go_travel_to_dk`). The
/// generic `_to_` form is deliberately NOT matched -- it is dominated by false
/// positives (`spoke_to_darth_marr`, `go_to_library`, `return_to_ship`). Known
/// destination abbreviations are expanded (`dk` -> `dromund_kaas`). The caller
/// validates `{dest}` against the planet anchor map, so any non-planet remainder
/// is dropped there.
pub(crate) fn planet_transit_dest(s: &str) -> Option<String> {
    // Prefer the longer verb so `traveled_to_` is not truncated by `travel_to_`.
    let after = s
        .split_once("traveled_to_")
        .or_else(|| s.split_once("travel_to_"))
        .map(|(_, rest)| rest)?;
    let dest = after.strip_prefix("the_").unwrap_or(after);
    if dest.is_empty()
        || !dest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
    {
        return None;
    }
    Some(match dest {
        "dk" => "dromund_kaas".to_string(),
        _ => dest.to_string(),
    })
}
