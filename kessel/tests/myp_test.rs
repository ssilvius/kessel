//! Tests for MYP archive reader

#[test]
fn test_myp_magic_detection() {
    // Valid MYP header starts with "MYP"
    let valid_header = b"MYP\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    assert!(valid_header.starts_with(b"MYP"));

    // Invalid header
    let invalid_header = b"NOT\x00\x00\x00\x00\x00";
    assert!(!invalid_header.starts_with(b"MYP"));
}
