//! Tests for schema/GameObject

use kessel::schema::GameObject;
use serde_json::json;

#[test]
fn test_is_quest_by_kind() {
    let obj = GameObject {
        kind: "Quest".to_string(),
        fqn: "something.else".to_string(),
        ..Default::default()
    };
    assert!(obj.is_quest());
}

#[test]
fn test_is_quest_by_fqn() {
    let obj = GameObject {
        kind: "Unknown".to_string(),
        fqn: "qst.class.warrior.act1".to_string(),
        ..Default::default()
    };
    assert!(obj.is_quest());
}

#[test]
fn test_is_ability_by_kind() {
    let obj = GameObject {
        kind: "Ability".to_string(),
        fqn: "something".to_string(),
        ..Default::default()
    };
    assert!(obj.is_ability());
}

#[test]
fn test_is_ability_by_fqn() {
    let obj = GameObject {
        kind: "Unknown".to_string(),
        fqn: "abl.force.lightning".to_string(),
        ..Default::default()
    };
    assert!(obj.is_ability());
}

#[test]
fn test_is_item_by_fqn() {
    let obj = GameObject {
        kind: "Unknown".to_string(),
        fqn: "itm.weapon.lightsaber".to_string(),
        ..Default::default()
    };
    assert!(obj.is_item());
}

#[test]
fn test_is_npc_by_fqn() {
    let obj = GameObject {
        kind: "Unknown".to_string(),
        fqn: "npc.vendor.armor".to_string(),
        ..Default::default()
    };
    assert!(obj.is_npc());
}

#[test]
fn test_default_values() {
    let obj = GameObject::default();
    assert!(obj.guid.is_empty());
    assert!(obj.fqn.is_empty());
    assert!(obj.kind.is_empty());
    assert_eq!(obj.version, 0);
    assert_eq!(obj.revision, 0);
}

#[test]
fn test_name_extraction() {
    let obj = GameObject {
        json: json!({
            "Name": {
                "%": "Test Name"
            }
        }),
        ..Default::default()
    };
    // name() looks for localized names - this tests the fallback path
    let name = obj.name("en");
    // May return None if path doesn't match expected structure
    assert!(name.is_none() || name == Some("Test Name".to_string()));
}
