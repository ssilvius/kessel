//! SWTOR Filename Hash Algorithm
//!
//! Computes the 64-bit filename hash used in MYP archives.
//! Based on EasyMYP's SWTORHash implementation.
//!
//! Hash format: (ph << 32) | sh where ph=primary hash, sh=secondary hash

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Compute SWTOR filename hash
/// Returns (primary_hash, secondary_hash)
pub fn hash_filename(filename: &str) -> (u32, u32) {
    // Normalize: lowercase, forward slashes
    let normalized: String = filename.to_lowercase().replace('\\', "/");
    let bytes: Vec<u8> = normalized.bytes().collect();

    let len = bytes.len();
    // SWTOR hash seed (per EasyMYP/Jedipedia reverse engineering)
    let seed: u32 = 0xDEADBEEF;

    let mut ph: u32 = (len as u32).wrapping_add(seed);
    let mut sh: u32 = ph;
    let mut hash3: u32 = ph;

    let mut pos = 0;

    // Process 12-byte chunks
    while pos + 12 < len {
        let tmp1 = get_u32(&bytes, pos);
        let tmp2 = get_u32(&bytes, pos + 4).wrapping_add(sh);
        let tmp3 = get_u32(&bytes, pos + 8).wrapping_add(ph);

        let v12 = (tmp3 << 4) ^ (tmp3 >> 28) ^ hash3.wrapping_add(tmp1).wrapping_sub(tmp3);
        let v13 = tmp2.wrapping_add(tmp3);
        let v16 = v13.wrapping_add(v12);
        let v17 = (v12 << 6) ^ (v12 >> 26) ^ tmp2.wrapping_sub(v12);
        let v18 = (v17 >> 24) ^ v13.wrapping_sub(v17);
        let v20 = v16.wrapping_add(v17);
        let v21 = (v17 << 8) ^ v18;
        let v22 = (v21 << 16) ^ (v21 >> 16) ^ v16.wrapping_sub(v21);
        let v23 = v20.wrapping_add(v21);
        let v24 = (v22 >> 13) ^ (v22 << 19) ^ v20.wrapping_sub(v22);

        hash3 = v23.wrapping_add(v22);
        ph = (v24 << 4) ^ (v24 >> 28) ^ v23.wrapping_sub(v24);
        sh = hash3.wrapping_add(v24);

        pos += 12;
    }

    // Process remaining bytes
    let remaining = len - pos;

    match remaining {
        12 => {
            hash3 = hash3.wrapping_add(get_u32(&bytes, pos + 8));
            sh = sh.wrapping_add(get_u32(&bytes, pos + 4));
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        11 => {
            hash3 = hash3.wrapping_add((bytes[pos + 10] as u32) << 16);
            hash3 = hash3.wrapping_add(get_u16(&bytes, pos + 8) as u32);
            sh = sh.wrapping_add(get_u32(&bytes, pos + 4));
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        10 => {
            hash3 = hash3.wrapping_add(get_u16(&bytes, pos + 8) as u32);
            sh = sh.wrapping_add(get_u32(&bytes, pos + 4));
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        9 => {
            hash3 = hash3.wrapping_add(bytes[pos + 8] as u32);
            sh = sh.wrapping_add(get_u32(&bytes, pos + 4));
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        8 => {
            sh = sh.wrapping_add(get_u32(&bytes, pos + 4));
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        7 => {
            sh = sh.wrapping_add((bytes[pos + 6] as u32) << 16);
            sh = sh.wrapping_add(get_u16(&bytes, pos + 4) as u32);
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        6 => {
            sh = sh.wrapping_add(get_u16(&bytes, pos + 4) as u32);
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        5 => {
            sh = sh.wrapping_add(bytes[pos + 4] as u32);
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        4 => {
            ph = ph.wrapping_add(get_u32(&bytes, pos));
        }
        3 => {
            ph = ph.wrapping_add((bytes[pos + 2] as u32) << 16);
            ph = ph.wrapping_add(get_u16(&bytes, pos) as u32);
        }
        2 => {
            ph = ph.wrapping_add(get_u16(&bytes, pos) as u32);
        }
        1 => {
            ph = ph.wrapping_add(bytes[pos] as u32);
        }
        0 => {
            return (ph, sh);
        }
        _ => {}
    }

    // Final mixing
    let v52 = (sh ^ ph).wrapping_sub((sh << 14) ^ (sh >> 18));
    let v53 = (hash3 ^ v52).wrapping_sub((v52 << 11) ^ (v52 >> 21));
    let v54 = (v53 ^ sh).wrapping_sub((v53 >> 7) ^ (v53 << 25));
    let v55 = (v54 ^ v52).wrapping_sub((v54 << 16) ^ (v54 >> 16));
    let v56 = (v53 ^ v55).wrapping_sub((v55 << 4) ^ (v55 >> 28));
    sh = (v56 ^ v54).wrapping_sub((v56 << 14) ^ (v56 >> 18));
    ph = (sh ^ v55).wrapping_sub((sh >> 8) ^ (sh << 24));

    (ph, sh)
}

/// Combine primary and secondary hash into 64-bit archive hash
pub fn combine_hash(ph: u32, sh: u32) -> u64 {
    ((ph as u64) << 32) | (sh as u64)
}

fn get_u32(bytes: &[u8], pos: usize) -> u32 {
    if pos + 4 <= bytes.len() {
        u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
    } else {
        0
    }
}

fn get_u16(bytes: &[u8], pos: usize) -> u16 {
    if pos + 2 <= bytes.len() {
        u16::from_le_bytes([bytes[pos], bytes[pos + 1]])
    } else {
        0
    }
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

    pub fn len(&self) -> usize {
        self.hash_to_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hash_to_path.is_empty()
    }
}

impl Default for HashDictionary {
    fn default() -> Self {
        Self::new()
    }
}

// Note: hash_filename() is available for future use but currently unused
// since we load pre-computed hashes from the Jedipedia dictionary file.
