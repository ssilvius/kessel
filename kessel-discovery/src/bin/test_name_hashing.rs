//! Test whether MAPPINGS.md CC hashes are hashes of property names.

use serde_json::Value;
use std::fs;

// Try a few common hash functions to see if any matches.
fn fnv1a_32(s: &[u8]) -> u32 {
    let mut h: u32 = 0x811C9DC5;
    for &b in s {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

fn fnv1_32(s: &[u8]) -> u32 {
    let mut h: u32 = 0x811C9DC5;
    for &b in s {
        h = h.wrapping_mul(0x01000193);
        h ^= b as u32;
    }
    h
}

fn djb2(s: &[u8]) -> u32 {
    let mut h: u32 = 5381;
    for &b in s {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    h
}

fn djb2_xor(s: &[u8]) -> u32 {
    let mut h: u32 = 5381;
    for &b in s {
        h = h.wrapping_mul(33) ^ (b as u32);
    }
    h
}

fn sdbm(s: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in s {
        h = (b as u32)
            .wrapping_add(h.wrapping_shl(6))
            .wrapping_add(h.wrapping_shl(16))
            .wrapping_sub(h);
    }
    h
}

fn crc32(s: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in s {
        let mut c = crc ^ (b as u32);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB88320
            } else {
                c >> 1
            };
        }
        crc = (crc >> 8) ^ c;
    }
    !crc
}

fn try_all(s: &str, target: u32) -> Option<&'static str> {
    if fnv1a_32(s.as_bytes()) == target {
        return Some("FNV-1a");
    }
    if fnv1_32(s.as_bytes()) == target {
        return Some("FNV-1");
    }
    if djb2(s.as_bytes()) == target {
        return Some("djb2");
    }
    if djb2_xor(s.as_bytes()) == target {
        return Some("djb2-xor");
    }
    if sdbm(s.as_bytes()) == target {
        return Some("sdbm");
    }
    if crc32(s.as_bytes()) == target {
        return Some("crc32");
    }
    None
}

fn main() -> anyhow::Result<()> {
    let targets: Vec<(&str, u32)> = vec![
        ("6F6FAE37", 0x6F6FAE37),
        ("17E2840B", 0x17E2840B),
        ("E4AFDD03", 0xE4AFDD03),
        ("0CCD312D", 0x0CCD312D),
        ("9D4BD719", 0x9D4BD719),
    ];

    // Candidate names
    let candidates: Vec<&str> = vec![
        "AbilityRef",
        "abilityRef",
        "ability_ref",
        "AbilityID",
        "StringRef",
        "stringRef",
        "string_ref",
        "StringID",
        "EffectRef",
        "effectRef",
        "effect_ref",
        "TalentRef",
        "talentRef",
        "Anchor",
        "anchor",
        "EffectAnchor",
        "effectAnchor",
        "TalAbilityRef",
        "tal.AbilityRef",
        "talAbilityRef",
        "talStringRef",
        "talEffectAnchor",
        "abilities",
        "ability",
        "strings",
        "effects",
        "abl",
        "tal",
    ];

    // Also pull enum names from the 748-enum dict
    let enums: Vec<Value> =
        serde_json::from_str(&fs::read_to_string("/tmp/client-gom-dict.json")?)?;
    let enum_names: Vec<String> = enums
        .iter()
        .filter_map(|e| e["name"].as_str().map(String::from))
        .collect();

    println!("Testing hash candidates against MAPPINGS.md CC hashes...\n");
    for (label, target) in &targets {
        let mut found = false;
        for c in &candidates {
            if let Some(algo) = try_all(c, *target) {
                println!("  {}: matched '{}' via {}", label, c, algo);
                found = true;
            }
        }
        for n in &enum_names {
            if let Some(algo) = try_all(n, *target) {
                println!("  {}: matched ENUM '{}' via {}", label, n, algo);
                found = true;
            }
        }
        if !found {
            println!(
                "  {}: NO MATCH from {} candidates + {} enum names",
                label,
                candidates.len(),
                enum_names.len()
            );
        }
    }

    Ok(())
}
