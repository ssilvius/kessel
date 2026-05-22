//! SCPT compiled-script file format.
//!
//! Files at `/resources/systemgenerated/compilednative/<numeric_id>.scpt` use
//! a constant-key XOR stream cipher over a 37-byte plaintext header. Verified
//! by sub-agent B across all 1,196 SCPT entries in v7.x archives (legion
//! reflection `019e4d62`).
//!
//! ## Header (37 bytes, plaintext)
//!
//! | offset | size | field          | meaning                           |
//! |-------:|-----:|----------------|-----------------------------------|
//! | 0x00   | 4    | magic          | `SCPT`                            |
//! | 0x04   | 4    | version        | `06 00 05 00` (LE) on v7.x        |
//! | 0x08   | 8    | section_count  | constant `1`                      |
//! | 0x10   | 8    | numeric_id     | u64 LE -- matches filename        |
//! | 0x18   | 8    | constant_marker| always `0x0000_0000_0000_0201`    |
//! | 0x20   | 1    | pad            | `0x00`                            |
//! | 0x21   | 3    | body_size      | u24 LE                            |
//! | 0x24   | 1    | pad            | `0x00`                            |
//!
//! ## Body cipher
//!
//! Constant XOR keystream, no per-file derivation:
//! `plaintext[i] = ciphertext[i] XOR ((0x43 + 11 * i) % 256)`
//!
//! ## What this is NOT
//!
//! SCPT bodies decrypt to native x86-64 UI/SFX code, not gameplay-formula data
//! or Pawn bytecode. Decoder ships as a utility for future UI-string scans
//! (cnv/qst/mpn cross-references); no spice table consumes SCPT bodies today.

use anyhow::{bail, Result};

const SCPT_MAGIC: [u8; 4] = *b"SCPT";
const SCPT_HEADER_SIZE: usize = 37;
const CIPHER_START: u8 = 0x43;
const CIPHER_STEP: u8 = 11;

/// SCPT plaintext header (37 bytes, uniform across all known files).
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct ScptHeader {
    pub version: u32,
    pub section_count: u64,
    pub numeric_id: u64,
    pub constant_marker: u64,
    pub body_size: u32,
}

impl ScptHeader {
    /// Parse the 37-byte SCPT header from the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < SCPT_HEADER_SIZE {
            bail!(
                "SCPT header truncated: got {} bytes, need {}",
                bytes.len(),
                SCPT_HEADER_SIZE
            );
        }
        if bytes[..4] != SCPT_MAGIC {
            bail!("invalid SCPT magic: {:02X?}", &bytes[..4]);
        }
        let body_size = u32::from_le_bytes([bytes[0x21], bytes[0x22], bytes[0x23], 0]);
        Ok(Self {
            version: u32::from_le_bytes(bytes[4..8].try_into().expect("4..8")),
            section_count: u64::from_le_bytes(bytes[8..16].try_into().expect("8..16")),
            numeric_id: u64::from_le_bytes(bytes[16..24].try_into().expect("16..24")),
            constant_marker: u64::from_le_bytes(bytes[24..32].try_into().expect("24..32")),
            body_size,
        })
    }
}

/// Decrypt an SCPT body using the verified XOR keystream.
///
/// `ciphertext` is the bytes following the 37-byte header. Output has the
/// same length.
#[allow(dead_code)]
pub fn decrypt_body(ciphertext: &[u8]) -> Vec<u8> {
    ciphertext
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let key = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(i as u8));
            b ^ key
        })
        .collect()
}

/// Parse the header and decrypt the body in one call.
///
/// Returns an error if the body size in the header does not match
/// `file_bytes.len() - 37`.
#[allow(dead_code)]
pub fn parse_and_decrypt(file_bytes: &[u8]) -> Result<(ScptHeader, Vec<u8>)> {
    let header = ScptHeader::parse(file_bytes)?;
    let body = &file_bytes[SCPT_HEADER_SIZE..];
    if body.len() != header.body_size as usize {
        bail!(
            "body size mismatch: header says {}, file has {}",
            header.body_size,
            body.len()
        );
    }
    Ok((header, decrypt_body(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_scpt(numeric_id: u64, body: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SCPT_HEADER_SIZE + body.len());
        buf.extend_from_slice(&SCPT_MAGIC);
        buf.extend_from_slice(&0x0006_0005u32.to_le_bytes());
        buf.extend_from_slice(&1u64.to_le_bytes());
        buf.extend_from_slice(&numeric_id.to_le_bytes());
        buf.extend_from_slice(&0x0000_0000_0000_0201u64.to_le_bytes());
        buf.push(0x00);
        let size_bytes = (body.len() as u32).to_le_bytes();
        buf.extend_from_slice(&size_bytes[..3]);
        buf.push(0x00);
        // Encrypt the body so decrypt round-trips correctly
        for (i, &b) in body.iter().enumerate() {
            let key = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(i as u8));
            buf.push(b ^ key);
        }
        buf
    }

    #[test]
    fn parses_header_fields() {
        let bytes = build_scpt(0xDEAD_BEEF_CAFE_BABE, b"hello");
        let header = ScptHeader::parse(&bytes).expect("parse");
        assert_eq!(header.numeric_id, 0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(header.section_count, 1);
        assert_eq!(header.constant_marker, 0x0000_0000_0000_0201);
        assert_eq!(header.body_size, 5);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = build_scpt(0, b"x");
        bytes[0] = b'X';
        let err = ScptHeader::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("invalid SCPT magic"));
    }

    #[test]
    fn rejects_truncated_header() {
        let bytes = b"SCPT\x06\x00\x05\x00";
        let err = ScptHeader::parse(bytes).unwrap_err();
        assert!(format!("{err}").contains("truncated"));
    }

    #[test]
    fn decrypt_round_trips_for_first_bytes() {
        // Verify against the hand-computed keystream for i=0..3.
        // i=0: key = 0x43
        // i=1: key = 0x43 + 11 = 0x4E
        // i=2: key = 0x43 + 22 = 0x59
        // i=3: key = 0x43 + 33 = 0x64
        let plain = [0xAA, 0xBB, 0xCC, 0xDD];
        let cipher: Vec<u8> = plain
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let key = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(i as u8));
                b ^ key
            })
            .collect();
        assert_eq!(
            cipher,
            vec![0xAA ^ 0x43, 0xBB ^ 0x4E, 0xCC ^ 0x59, 0xDD ^ 0x64]
        );
        let recovered = decrypt_body(&cipher);
        assert_eq!(recovered, plain);
    }

    #[test]
    fn parse_and_decrypt_returns_plaintext_body() {
        let plain = b"hello world!";
        let bytes = build_scpt(42, plain);
        let (header, body) = parse_and_decrypt(&bytes).expect("parse_and_decrypt");
        assert_eq!(header.numeric_id, 42);
        assert_eq!(body, plain);
    }

    #[test]
    fn body_size_mismatch_errors() {
        let mut bytes = build_scpt(0, b"abcd");
        // Lie about the body size in the header
        bytes[0x21] = 99;
        let err = parse_and_decrypt(&bytes).unwrap_err();
        assert!(format!("{err}").contains("body size mismatch"));
    }

    #[test]
    fn decrypt_empty_body_is_empty() {
        assert!(decrypt_body(&[]).is_empty());
    }

    #[test]
    fn keystream_wraps_at_256_bytes() {
        // i=23 -> 0x43 + 23*11 = 0x43 + 0xFD = 0x140 -> u8 wraps to 0x40
        let key_23 = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(23));
        assert_eq!(key_23, 0x40);
        // i=255 -> further wrapping must not panic
        let _ = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(255));
    }
}
