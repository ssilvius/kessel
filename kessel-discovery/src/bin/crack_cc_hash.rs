//! Spike (#144): brute-force the SWTOR CC field-name hash function.
//!
//! The 4-byte CC hashes in GOM payloads (`CC 6F 6F AE 37`, `CC 17 E2 84 0B`,
//! `CC E4 AF DD 03`, `CC 0C CD 31 2D` from MAPPINGS.md) live in a separate
//! proprietary Bioware namespace and are NOT lookups into client.gom (legion
//! `019e4d75`). Sub-agent F (`019e4d77`) tested 16 standard hash functions
//! against candidate names -- none match.
//!
//! This binary frames the next attack: pair each known CC hash with N
//! likely field-name candidates (drawn from structural position guesses) and
//! try them under M hash-function variants. If any (name, function) pair
//! matches all 4 known hashes simultaneously, the function is identified.
//!
//! Run:
//!   cargo run -p kessel-discovery --bin crack_cc_hash --release
//!
//! Negative result: prints `no candidate function matched`. Update the
//! function/name lists and re-run.

use std::collections::HashMap;

/// Known CC hash → suspected purpose (from MAPPINGS.md talent-format research).
const KNOWN_HASHES: &[(u32, &str)] = &[
    (
        0x6F6F_AE37,
        "stringRef -- precedes str.tal/str.abl/etc. refs",
    ),
    (
        0x17E2_840B,
        "abilityRef -- precedes CF E0 GUID for abl.* targets",
    ),
    (
        0xE4AF_DD03,
        "effectField -- co-occurs with CF40 D954FB05 (effect block header)",
    ),
    (0x0CCD_312D, "unknown -- 1 per talent payload, position TBD"),
];

/// Candidate field-name strings. Each is hashed under every candidate
/// function; matches across multiple known CC hashes flag the right function.
///
/// Sourced from structural-position guesses:
/// - Talent class (D954FB01) has 7 declared properties per Agent D.
/// - Likely names: stringRef, abilityRef, effectRef, name, displayName,
///   nameStringId, descriptionStringId, iconName, effects, abilities.
const CANDIDATE_NAMES: &[&str] = &[
    "stringRef",
    "abilityRef",
    "effectRef",
    "effectField",
    "stringTableRef",
    "stringId",
    "nameStringId",
    "descriptionStringId",
    "displayNameStringId",
    "iconName",
    "name",
    "displayName",
    "effects",
    "abilities",
    "icon",
    "level",
    "rank",
    "talentName",
    "stringRefId",
    "abilityRefId",
    "fieldStringRef",
    "fieldAbilityRef",
    "fieldEffect",
];

/// Variants of class-namespace prefixes that may be prepended before hashing.
const NAMESPACE_PREFIXES: &[&str] = &[
    "",
    "Talent.",
    "talent.",
    "GOM.",
    "Bioware.",
    "field.",
    "Field.",
    "prop.",
    "Property.",
];

fn main() {
    eprintln!(
        "Attacking {} known CC hashes against {} candidate names x {} prefixes \
         x N hash functions.",
        KNOWN_HASHES.len(),
        CANDIDATE_NAMES.len(),
        NAMESPACE_PREFIXES.len(),
    );

    let mut matches: HashMap<&'static str, Vec<(u32, String)>> = HashMap::new();

    for (known_hash, purpose) in KNOWN_HASHES {
        for prefix in NAMESPACE_PREFIXES {
            for name in CANDIDATE_NAMES {
                let candidate = format!("{prefix}{name}");
                for (fn_name, hash_fn) in hash_function_table() {
                    let computed = hash_fn(candidate.as_bytes());
                    if computed == *known_hash {
                        matches
                            .entry(fn_name)
                            .or_default()
                            .push((*known_hash, candidate.clone()));
                        println!(
                            "MATCH: fn={fn_name} candidate={candidate:?} \
                             hash=0x{known_hash:08X} ({purpose})"
                        );
                    }
                }
            }
        }
    }

    if matches.is_empty() {
        println!(
            "no candidate function matched any known CC hash. \
             Next: extend CANDIDATE_NAMES with property names recovered from \
             gameplay observation, or extend hash_function_table() with custom \
             polynomial variants (CRC32 with non-standard polys, FNV with \
             custom seed/prime, Murmur3 with custom seed). See issue #144."
        );
        std::process::exit(1);
    }

    println!();
    println!("=== summary ===");
    for (fn_name, hits) in &matches {
        println!(
            "{fn_name}: {} match(es) -> {:?}",
            hits.len(),
            hits.iter().map(|(_, n)| n).collect::<Vec<_>>()
        );
    }
}

/// Hash-function variants to try. Each maps a byte slice to a u32.
///
/// Sub-agent F already ruled out: CRC32, FNV-1, FNV-1a, djb2, sdbm, Murmur3,
/// Jenkins OAAT, hashlittle2-c, hashlittle2-b, MD5/SHA1/SHA256 first-4-LE,
/// MD5/SHA1/SHA256 first-4-BE. This table re-lists those for reproducibility
/// + adds Bioware-internal variant guesses.
type HashFn = fn(&[u8]) -> u32;
fn hash_function_table() -> Vec<(&'static str, HashFn)> {
    vec![
        ("fnv1a_32_le", |b| fnv1a_32(b)),
        ("fnv1a_32_be", |b| fnv1a_32(b).swap_bytes()),
        ("djb2_le", |b| djb2(b) as u32),
        ("sdbm_le", |b| sdbm(b) as u32),
        ("crc32_ieee", crc32_ieee),
        ("crc32c", crc32c),
        ("fnv1a_with_swtor_seed", |b| fnv1a_seeded(b, 0x5057_544F)),
        ("fnv1a_with_bioware_seed", |b| fnv1a_seeded(b, 0x4257_4F52)),
    ]
}

const FNV_PRIME_32: u32 = 16_777_619;
const FNV_OFFSET_32: u32 = 2_166_136_261;

fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut h = FNV_OFFSET_32;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME_32);
    }
    h
}

fn fnv1a_seeded(bytes: &[u8], seed: u32) -> u32 {
    let mut h = seed;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME_32);
    }
    h
}

fn djb2(bytes: &[u8]) -> u64 {
    let mut h: u64 = 5381;
    for &b in bytes {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

fn sdbm(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0;
    for &b in bytes {
        h = (b as u64)
            .wrapping_add(h.wrapping_shl(6))
            .wrapping_add(h.wrapping_shl(16))
            .wrapping_sub(h);
    }
    h
}

const CRC32_IEEE_POLY: u32 = 0xEDB8_8320;
const CRC32C_POLY: u32 = 0x82F6_3B78;

fn crc32_generic(bytes: &[u8], poly: u32) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn crc32_ieee(bytes: &[u8]) -> u32 {
    crc32_generic(bytes, CRC32_IEEE_POLY)
}

fn crc32c(bytes: &[u8]) -> u32 {
    crc32_generic(bytes, CRC32C_POLY)
}
