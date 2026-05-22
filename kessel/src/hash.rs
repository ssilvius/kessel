//! Hash utilities for SWTOR data
//!
//! - SWTOR filename hash: 64-bit hash used in MYP archives
//! - game_id: sha256(fqn:guid)[0:16] -- unique per object-instance per extraction
//! - stable_id: sha256(fqn)[0:16] -- cross-patch identity (FQN is Bioware's
//!   semantic identity; survives patches)
//! - payload_hash: sha256(payload_bytes)[0:16] -- change detector for delta joins
//! - Icon ID: sha256(name)[0:16] -- deterministic icon filenames

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Compute game_id from FQN + GUID.
/// Returns 16-character hex string: sha256(fqn:guid)[0:16].
///
/// Unique per object-instance per extraction. The compound is required because
/// neither field is unique-and-stable on its own:
/// - FQN is not unique in raw extraction (canonical objects + stub references
///   share an FQN).
/// - GUID shifts on every game patch.
///
/// game_id is the join key for the current extraction. For cross-patch identity
/// tracking, use `compute_stable_id(fqn)` instead -- that hash is stable across
/// patches but only unique post-dedup.
pub fn compute_game_id(fqn: &str, guid: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fqn);
    hasher.update(b":");
    hasher.update(guid);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Compute stable_id from FQN alone.
/// Returns 16-character hex string: sha256(fqn)[0:16].
///
/// Stable across patch versions -- FQN is Bioware's semantic identity and
/// survives patches (a rename is a real semantic change, not drift). Unique
/// only post-dedup (`mark_canonical_by_fqn`); not suitable as a PK on raw
/// extraction.
///
/// Use for cross-version delta joins: `JOIN ... USING (stable_id)` finds the
/// same logical object across two extractions even when its GUID has shifted.
pub fn compute_stable_id(fqn: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fqn);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Compute payload_hash from raw GOM payload bytes.
/// Returns 16-character hex string: sha256(payload)[0:16].
///
/// Not an identity. Used to detect "did this object's data change between
/// extractions" when joined to `stable_id`.
pub fn compute_payload_hash(payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Compute icon ID from icon name
/// Returns 16-character hex string: sha256(name)[0:16]
///
/// Used for cache-friendly icon filenames.
pub fn compute_icon_id(name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Combine primary and secondary hash into 64-bit archive hash
pub fn combine_hash(ph: u32, sh: u32) -> u64 {
    ((ph as u64) << 32) | (sh as u64)
}

/// Hash dictionary mapping 64-bit hash to filepath
pub struct HashDictionary {
    hash_to_path: HashMap<u64, String>,
}

impl HashDictionary {
    pub fn new() -> Self {
        Self {
            hash_to_path: HashMap::new(),
        }
    }

    /// Load hash file in EasyMYP format: ph#sh#filepath#CRC
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<usize> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut count = 0;

        for line in reader.lines() {
            let line = line?;
            let parts: Vec<&str> = line.split('#').collect();
            if parts.len() >= 3 {
                // Parse hex hashes
                if let (Ok(ph), Ok(sh)) = (
                    u32::from_str_radix(parts[0], 16),
                    u32::from_str_radix(parts[1], 16),
                ) {
                    let hash = combine_hash(ph, sh);
                    let filepath = parts[2].to_string();
                    self.hash_to_path.insert(hash, filepath);
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Look up filepath by hash
    pub fn get(&self, hash: u64) -> Option<&String> {
        self.hash_to_path.get(&hash)
    }

    /// Check if path matches a pattern
    pub fn paths_matching(&self, pattern: &str) -> Vec<(u64, &String)> {
        self.hash_to_path
            .iter()
            .filter(|(_, path)| path.contains(pattern))
            .map(|(hash, path)| (*hash, path))
            .collect()
    }
}

impl Default for HashDictionary {
    fn default() -> Self {
        Self::new()
    }
}

// ----- Bob Jenkins lookup3 hashlittle2 ---------------------------------------
//
// SWTOR's MYP archive filename hash is Bob Jenkins' `hashlittle2` from
// https://burtleburtle.net/bob/c/lookup3.c
// (verified 2026-05-21 against `~/swtor/data/hashes_filename.txt`).
//
// EasyMYP format stores `PH#SH#path` where:
//   PH = hashlittle2(path).primary
//   SH = hashlittle2(path).secondary
//
// `swtor_filename_hash(path) -> u64` combines them as `(SH << 32) | PH`.
//
// Case-preserving, UTF-8, no null terminator. Pure-Rust translation; no FFI.

#[inline(always)]
#[allow(dead_code)]
fn rot(x: u32, k: u32) -> u32 {
    x.rotate_left(k)
}

#[inline(always)]
#[allow(dead_code)]
fn mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    a = a.wrapping_sub(c);
    a ^= rot(c, 4);
    c = c.wrapping_add(b);
    b = b.wrapping_sub(a);
    b ^= rot(a, 6);
    a = a.wrapping_add(c);
    c = c.wrapping_sub(b);
    c ^= rot(b, 8);
    b = b.wrapping_add(a);
    a = a.wrapping_sub(c);
    a ^= rot(c, 16);
    c = c.wrapping_add(b);
    b = b.wrapping_sub(a);
    b ^= rot(a, 19);
    a = a.wrapping_add(c);
    c = c.wrapping_sub(b);
    c ^= rot(b, 4);
    b = b.wrapping_add(a);
    (a, b, c)
}

#[inline(always)]
#[allow(dead_code)]
fn final_mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    c ^= b;
    c = c.wrapping_sub(rot(b, 14));
    a ^= c;
    a = a.wrapping_sub(rot(c, 11));
    b ^= a;
    b = b.wrapping_sub(rot(a, 25));
    c ^= b;
    c = c.wrapping_sub(rot(b, 16));
    a ^= c;
    a = a.wrapping_sub(rot(c, 4));
    b ^= a;
    b = b.wrapping_sub(rot(a, 14));
    c ^= b;
    c = c.wrapping_sub(rot(b, 24));
    (a, b, c)
}

#[inline(always)]
#[allow(dead_code)]
fn load_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// Bob Jenkins lookup3 `hashlittle2`. Returns the `(b, c)` output pair as
/// `(primary, secondary)` u32s.
///
/// `initval` seeds the primary half, `initval2` the secondary half. Most
/// callers use `(0, 0)`.
#[allow(dead_code)]
pub fn hashlittle2(key: &[u8], initval: u32, initval2: u32) -> (u32, u32) {
    let length = key.len() as u32;
    let mut a = 0xdeadbeef_u32.wrapping_add(length).wrapping_add(initval);
    let mut b = a;
    let mut c = a.wrapping_add(initval2);

    let mut offset = 0;
    let mut remaining = key.len();

    // Process full 12-byte chunks
    while remaining > 12 {
        a = a.wrapping_add(load_u32_le(key, offset));
        b = b.wrapping_add(load_u32_le(key, offset + 4));
        c = c.wrapping_add(load_u32_le(key, offset + 8));
        let r = mix(a, b, c);
        a = r.0;
        b = r.1;
        c = r.2;
        offset += 12;
        remaining -= 12;
    }

    // Handle the last 0..12 bytes
    match remaining {
        12 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(load_u32_le(key, offset + 4));
            c = c.wrapping_add(load_u32_le(key, offset + 8));
        }
        11 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(load_u32_le(key, offset + 4));
            c = c.wrapping_add(
                (key[offset + 8] as u32)
                    | ((key[offset + 9] as u32) << 8)
                    | ((key[offset + 10] as u32) << 16),
            );
        }
        10 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(load_u32_le(key, offset + 4));
            c = c.wrapping_add((key[offset + 8] as u32) | ((key[offset + 9] as u32) << 8));
        }
        9 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(load_u32_le(key, offset + 4));
            c = c.wrapping_add(key[offset + 8] as u32);
        }
        8 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(load_u32_le(key, offset + 4));
        }
        7 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(
                (key[offset + 4] as u32)
                    | ((key[offset + 5] as u32) << 8)
                    | ((key[offset + 6] as u32) << 16),
            );
        }
        6 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add((key[offset + 4] as u32) | ((key[offset + 5] as u32) << 8));
        }
        5 => {
            a = a.wrapping_add(load_u32_le(key, offset));
            b = b.wrapping_add(key[offset + 4] as u32);
        }
        4 => {
            a = a.wrapping_add(load_u32_le(key, offset));
        }
        3 => {
            a = a.wrapping_add(
                (key[offset] as u32)
                    | ((key[offset + 1] as u32) << 8)
                    | ((key[offset + 2] as u32) << 16),
            );
        }
        2 => {
            a = a.wrapping_add((key[offset] as u32) | ((key[offset + 1] as u32) << 8));
        }
        1 => {
            a = a.wrapping_add(key[offset] as u32);
        }
        0 => return (b, c),
        _ => unreachable!(),
    }

    let r = final_mix(a, b, c);
    (r.1, r.2)
}

/// Compute SWTOR's 64-bit MYP archive filename hash for a path.
///
/// `swtor_filename_hash(path) == (SH << 32) | PH` where `(PH, SH) =
/// hashlittle2(path, 0, 0)`. Matches the `PH#SH#path` entries in
/// `hashes_filename.txt`.
#[allow(dead_code)]
pub fn swtor_filename_hash(path: &str) -> u64 {
    let (ph, sh) = hashlittle2(path.as_bytes(), 0, 0);
    ((sh as u64) << 32) | (ph as u64)
}

#[cfg(test)]
mod hashlittle2_tests {
    use super::*;

    /// Verified against `~/swtor/data/hashes_filename.txt` real entries.
    /// Format from the dictionary: `PH#SH#path#CRC`.
    #[test]
    fn matches_real_archive_paths() {
        // Real PH/SH pairs taken from hashes_filename.txt.
        let cases: &[(&str, u32, u32)] = &[
            // /resources/en-us/str/abl.stb
            ("/resources/en-us/str/abl.stb", 0x8154956D, 0x54305B3B),
            // /resources/systemgenerated/client.gom
            (
                "/resources/systemgenerated/client.gom",
                0x6107069D,
                0xB7C70D58,
            ),
        ];
        for (path, exp_ph, exp_sh) in cases {
            let (ph, sh) = hashlittle2(path.as_bytes(), 0, 0);
            assert_eq!(
                ph, *exp_ph,
                "primary hash mismatch for {path}: got {ph:08X}, want {exp_ph:08X}"
            );
            assert_eq!(
                sh, *exp_sh,
                "secondary hash mismatch for {path}: got {sh:08X}, want {exp_sh:08X}"
            );
            let combined = swtor_filename_hash(path);
            assert_eq!(combined, ((*exp_sh as u64) << 32) | (*exp_ph as u64));
        }
    }

    #[test]
    fn empty_input() {
        // For empty input the function returns (initval, initval2 + initval).
        // With (0, 0) seeds, that's just (0xdeadbeef, 0xdeadbeef).
        let (ph, sh) = hashlittle2(b"", 0, 0);
        assert_eq!(ph, 0xdeadbeef);
        assert_eq!(sh, 0xdeadbeef);
    }

    #[test]
    fn case_preserving() {
        let lower = swtor_filename_hash("/resources/en-us/str/abl.stb");
        let upper = swtor_filename_hash("/resources/EN-US/STR/abl.stb");
        assert_ne!(
            lower, upper,
            "case-preserving -- different case must hash differently"
        );
    }

    #[test]
    fn deterministic() {
        let a = swtor_filename_hash("/resources/systemgenerated/client.gom");
        let b = swtor_filename_hash("/resources/systemgenerated/client.gom");
        assert_eq!(a, b);
    }
}
