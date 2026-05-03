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
