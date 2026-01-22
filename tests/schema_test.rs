//! Tests for schema/GameObject

use kessel::pbuk::GomObject;
use kessel::schema::GameObject;

#[test]
fn test_default_values() {
    let obj = GameObject::default();
    assert!(obj.guid.is_empty());
    assert!(obj.fqn.is_empty());
    assert!(obj.kind.is_empty());
    assert!(obj.icon_name.is_none());
    assert_eq!(obj.version, 0);
    assert_eq!(obj.revision, 0);
}

// Helper to create a GomObject with specific payload
fn make_gom(fqn: &str, payload: Vec<u8>) -> GomObject {
    GomObject {
        fqn: fqn.to_string(),
        header: vec![0u8; 42], // 42-byte header with zeros (valid GUID = 0)
        payload,
    }
}

#[test]
fn test_icon_name_extraction_ability() {
    // Ability: icon at start of payload (first 60 bytes)
    // Pattern: 0x06 <length> <string>
    // extract_visual_ref requires: length > 4, length < 60
    let icon = b"abl_sw_rage_icon"; // 16 chars, contains underscore
    let mut payload = vec![0u8; 100];
    payload[17] = 0x06; // marker at offset 17
    payload[18] = icon.len() as u8; // length = 16
    payload[19..19 + icon.len()].copy_from_slice(icon);

    let gom = make_gom("abl.sith_warrior.rage", payload);
    let obj = GameObject::from_gom(&gom);

    assert_eq!(obj.icon_name, Some("abl_sw_rage_icon".to_string()));
    assert_eq!(obj.kind, "Ability");
}

#[test]
fn test_icon_name_extraction_talent() {
    // Talent: icon at end of payload (last 100 bytes)
    // Build payload with icon near the end
    let mut payload = vec![0u8; 250]; // lots of padding

    // Put icon ref near end (at offset ~278 like real talents)
    let icon_name = b"abl_bh_me_kolto_surge";
    let icon_offset = 278;
    payload.resize(icon_offset + 2 + icon_name.len() + 10, 0);
    payload[icon_offset] = 0x06; // marker
    payload[icon_offset + 1] = icon_name.len() as u8; // length
    payload[icon_offset + 2..icon_offset + 2 + icon_name.len()].copy_from_slice(icon_name);

    let gom = make_gom("tal.bounty_hunter.skill.utility.kolto_surge", payload);
    let obj = GameObject::from_gom(&gom);

    assert_eq!(obj.icon_name, Some("abl_bh_me_kolto_surge".to_string()));
    // Talents get kind mapped like other prefixes
    assert_eq!(obj.kind, "Talent");
}

#[test]
fn test_icon_name_no_match() {
    // Payload with no valid icon pattern
    let payload = vec![0u8; 100];
    let gom = make_gom("abl.test.ability", payload);
    let obj = GameObject::from_gom(&gom);

    assert!(obj.icon_name.is_none());
}

#[test]
fn test_icon_name_skips_str_prefix() {
    // Talent payload with "str.tal" strings that should be skipped
    let mut payload = vec![0u8; 200];

    // Add str.tal at offset 180 (should be skipped)
    payload[180] = 0x06;
    payload[181] = 7;
    payload[182..189].copy_from_slice(b"str.tal");

    // Add real icon at offset 190
    payload[190] = 0x06;
    payload[191] = 8;
    payload[192..200].copy_from_slice(b"railshot");

    let gom = make_gom("tal.test.talent", payload);
    let obj = GameObject::from_gom(&gom);

    // Should get railshot, not str.tal
    assert_eq!(obj.icon_name, Some("railshot".to_string()));
}
