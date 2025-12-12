//! Tests for PBUK/DBLB parser

use kessel::pbuk;

#[test]
fn test_is_pbuk_valid() {
    let data = b"PBUK\x01\x00\x00\x00\x10\x00\x00\x00";
    assert!(pbuk::is_pbuk(data));
}

#[test]
fn test_is_pbuk_invalid() {
    let data = b"NOTPBUK";
    assert!(!pbuk::is_pbuk(data));
}

#[test]
fn test_is_pbuk_too_short() {
    let data = b"PBU";
    assert!(!pbuk::is_pbuk(data));
}

#[test]
fn test_is_dblb_valid() {
    let data = b"DBLB\x00\x00\x00\x00";
    assert!(pbuk::is_dblb(data));
}

#[test]
fn test_is_dblb_invalid() {
    let data = b"NOTDBLB";
    assert!(!pbuk::is_dblb(data));
}

#[test]
fn test_parse_too_small_pbuk() {
    // PBUK without required DBLB structures should fail
    // Real SWTOR PBUK files require DBLB at offset 12 and 28
    let data = b"PBUK\x00\x00\x00\x00\x00\x00\x00\x00";
    let result = pbuk::parse(data);
    assert!(result.is_err()); // Too small, missing DBLB blocks
}

#[test]
fn test_parse_invalid_magic() {
    let data = b"NOTPBUK";
    let result = pbuk::parse(data);
    assert!(result.is_err());
}
