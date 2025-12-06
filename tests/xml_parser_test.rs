//! Tests for XML parser

use kessel::xml_parser;

#[test]
fn test_parse_simple_quest() {
    let xml = br#"<Quest GUID="12345" fqn="qst.test.example" Version="1" Revision="42">
        <Name>Test Quest</Name>
    </Quest>"#;

    let result = xml_parser::parse(xml);
    assert!(result.is_ok());

    let obj = result.unwrap();
    assert_eq!(obj.guid, "12345");
    assert_eq!(obj.fqn, "qst.test.example");
    assert_eq!(obj.kind, "Quest");
    assert_eq!(obj.version, 1);
    assert_eq!(obj.revision, 42);
}

#[test]
fn test_parse_ability() {
    let xml = br#"<Ability GUID="67890" fqn="abl.test.fireball" Version="2" Revision="10">
        <Cooldown>5</Cooldown>
    </Ability>"#;

    let result = xml_parser::parse(xml);
    assert!(result.is_ok());

    let obj = result.unwrap();
    assert_eq!(obj.guid, "67890");
    assert_eq!(obj.fqn, "abl.test.fireball");
    assert_eq!(obj.kind, "Ability");
}

#[test]
fn test_parse_uses_id_as_fallback() {
    let xml = br#"<Item GUID="11111" Id="itm.test.sword" Version="1" Revision="1">
    </Item>"#;

    let result = xml_parser::parse(xml);
    assert!(result.is_ok());

    let obj = result.unwrap();
    assert_eq!(obj.fqn, "itm.test.sword");
}

#[test]
fn test_parse_empty_element() {
    let xml = br#"<Npc GUID="99999" fqn="npc.test.guard" Version="1" Revision="1" />"#;

    let result = xml_parser::parse(xml);
    assert!(result.is_ok());

    let obj = result.unwrap();
    assert_eq!(obj.guid, "99999");
    assert_eq!(obj.kind, "Npc");
}

#[test]
fn test_parse_invalid_xml() {
    let xml = b"not valid xml at all";
    let result = xml_parser::parse(xml);
    // Should return default/empty object, not error
    assert!(result.is_ok());
}
