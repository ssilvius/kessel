//! Decoder for the GSF ship loadout slot-template singletons
//! (`conSpec_scff_equip_*`).
//!
//! Each template declares the component slots a GSF ship configuration of that
//! shape exposes. Major templates (`conSpec_scff_equip_maj_*`) carry the
//! weapon/shield/engine slots; minor templates (`conSpec_scff_equip_min_*`)
//! carry the four reactor/armor/sensor-class slots. The template-code suffix
//! spells the composition (e.g. `maj_PPSHE` = 2x Primary + Secondary + Shield +
//! Engine; `min_ACMR` = Armor/Capacitor/Magazine/Reactor).
//!
//! Wire format: the payload is a typed-value GOM stream; the slot names appear
//! as length-prefixed ASCII strings of the form
//! `conSlotEquipSCFF<SlotType>_<ordinal>`. Each slot is written twice (an
//! available-list copy and a default copy), so the decoder dedupes on
//! `(slot_type, ordinal)`. We scan for the fixed `conSlotEquipSCFF` marker
//! rather than walking the full type-tag stream -- the marker is unambiguous
//! and the surrounding framing carries no extra per-slot data.
//!
//! The ship -> template binding is NOT stored in the archive (the
//! `itm.spvp.ships.premium.*` payloads reference only shared item templates and
//! their appearance, not a loadout template); it is client-side and resolved by
//! ship class downstream. Issue #115 lineage.

/// One component slot within a loadout template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutSlot {
    /// Slot type token, e.g. `PrimaryWeapon`, `ShieldProjector`, `Armor`.
    pub slot_type: String,
    /// 1-based slot ordinal within its type (e.g. `PrimaryWeapon_1`,
    /// `PrimaryWeapon_2`).
    pub slot_ordinal: u32,
}

const MARKER: &[u8] = b"conSlotEquipSCFF";

/// Decode the distinct component slots declared by a `conSpec_scff_equip_*`
/// template payload. Returns slots in first-seen order, deduped on
/// `(slot_type, ordinal)`.
pub fn decode_loadout_slots(payload: &[u8]) -> Vec<LoadoutSlot> {
    let mut out: Vec<LoadoutSlot> = Vec::new();
    let mut i = 0usize;
    while i + MARKER.len() <= payload.len() {
        if &payload[i..i + MARKER.len()] != MARKER {
            i += 1;
            continue;
        }
        let mut j = i + MARKER.len();
        // Slot type: leading ASCII alphabetic run.
        let type_start = j;
        while j < payload.len() && payload[j].is_ascii_alphabetic() {
            j += 1;
        }
        // Expect the `_<ordinal>` suffix.
        if j >= payload.len() || payload[j] != b'_' {
            i += MARKER.len();
            continue;
        }
        let slot_type = String::from_utf8_lossy(&payload[type_start..j]).into_owned();
        j += 1; // skip '_'
        let ord_start = j;
        while j < payload.len() && payload[j].is_ascii_digit() {
            j += 1;
        }
        if j == ord_start {
            i += MARKER.len();
            continue;
        }
        let ordinal: u32 = String::from_utf8_lossy(&payload[ord_start..j])
            .parse()
            .unwrap_or(0);
        if slot_type.is_empty() || ordinal == 0 {
            i = j;
            continue;
        }
        let slot = LoadoutSlot {
            slot_type,
            slot_ordinal: ordinal,
        };
        if !out.contains(&slot) {
            out.push(slot);
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

    // Real `conSpec_scff_equip_maj_PPSHE` payload (spice-7.9.a-v1).
    const PPSHE_B64: &str = "EA3PQAAAAzKB/wwIBQIEBEQCRQFGAUgBzARKXVnDCAUCBAREAEUCRgRIA8wD5FAnqggFBwQERAIAAEUCAABGAgAASAIAAAEFIQEIBQcEBEQCAABFAgAARgIAAEgCAADMAzJViUwIBQcEBEQCAABFAgAARgIAAEgCAADMAnnPaHMGHGNvblNwZWNfc2NmZl9lcXVpcF9tYWpfUFBTSEXMC9eSTkYIAgUFBQFEAkQDRQRIBUbMEnBQ4QAIBQcEBEQCAgIBAQICRQIBAQEDRgIBAQEFSAIBAQEEzAEGM5DQCAYCBQXSGGNvblNsb3RFcXVpcFNDRkZFbmdpbmVfMQXSH2NvblNsb3RFcXVpcFNDRkZQcmltYXJ5V2VhcG9uXzEB0h9jb25TbG90RXF1aXBTQ0ZGUHJpbWFyeVdlYXBvbl8yAtIhY29uU2xvdEVxdWlwU0NGRlNlY29uZGFyeVdlYXBvbl8xA9IhY29uU2xvdEVxdWlwU0NGRlNoaWVsZFByb2plY3Rvcl8xBAEIAgYFBQEfY29uU2xvdEVxdWlwU0NGRlByaW1hcnlXZWFwb25fMQIfY29uU2xvdEVxdWlwU0NGRlByaW1hcnlXZWFwb25fMgMhY29uU2xvdEVxdWlwU0NGRlNlY29uZGFyeVdlYXBvbl8xBCFjb25TbG90RXF1aXBTQ0ZGU2hpZWxkUHJvamVjdG9yXzEFGGNvblNsb3RFcXVpcFNDRkZFbmdpbmVfMcweFFd9pwIFzQOElU6auAgFAwQERABFAEYASAA=";

    #[test]
    fn decodes_ppshe_major_template() {
        let payload = BASE64.decode(PPSHE_B64).expect("valid b64");
        let slots = decode_loadout_slots(&payload);
        let mut got: Vec<(String, u32)> = slots
            .into_iter()
            .map(|s| (s.slot_type, s.slot_ordinal))
            .collect();
        got.sort();
        let mut want = vec![
            ("Engine".to_string(), 1),
            ("PrimaryWeapon".to_string(), 1),
            ("PrimaryWeapon".to_string(), 2),
            ("SecondaryWeapon".to_string(), 1),
            ("ShieldProjector".to_string(), 1),
        ];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn dedupes_repeated_slot_copies() {
        // Two identical slot strings collapse to one row.
        let mut p = Vec::new();
        p.extend_from_slice(b"\x06conSlotEquipSCFFArmor_1");
        p.extend_from_slice(b"\x06conSlotEquipSCFFArmor_1");
        let slots = decode_loadout_slots(&p);
        assert_eq!(
            slots,
            vec![LoadoutSlot {
                slot_type: "Armor".to_string(),
                slot_ordinal: 1,
            }]
        );
    }

    #[test]
    fn ignores_payload_without_marker() {
        assert!(decode_loadout_slots(b"no slots here").is_empty());
    }
}
