//! Decode every GSF singleton prototype in the .tor archives.
//!
//! Walks the GOM type-tag stream emitting structured tokens, then groups
//! tokens into records using per-prototype boundary markers. Output: JSON to
//! docs/prototypes-decoded/gsf/<prototype>.json plus a coverage summary.
//!
//! Targeted prototypes:
//!   utlShipInfoPrototype, scffCrewPrototype, scffCrewPackagesPrototype,
//!   scFFComponentsCostPrototype, scFFComponentUpgradesCostPrototype,
//!   scFFColorSwatchesPrototype, scFFColorOptionsCostPrototype,
//!   scFFPatternsDefinitionProtoype, scFFPatternsCostPrototype,
//!   scFFPatternsTextureDataProtoype, gldFlagshipPrototype.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const TARGETS: &[&str] = &[
    "utlShipInfoPrototype",
    "scffCrewPrototype",
    "scffCrewPackagesPrototype",
    "scFFComponentsCostPrototype",
    "scFFComponentUpgradesCostPrototype",
    "scFFColorSwatchesPrototype",
    "scFFColorOptionsCostPrototype",
    "scFFPatternsDefinitionProtoype",
    "scFFPatternsCostPrototype",
    "scFFPatternsTextureDataProtoype",
    "gldFlagshipPrototype",
];

#[derive(Debug, Clone)]
enum Token {
    /// `06 <len> <ascii>` -- bare string.
    String { offset: usize, value: String },
    /// `01 06 <len> <ascii>` -- typed length-prefixed string.
    TypedString { offset: usize, value: String },
    /// `D2 01 <idx> <len> <ascii>` -- array element string.
    ArrayString {
        offset: usize,
        idx: u8,
        value: String,
    },
    /// `01 01 <byte>` -- typed u8.
    U8 { offset: usize, value: u8 },
    /// `01 02 <2 bytes LE>` -- typed u16.
    U16Le { offset: usize, value: u16 },
    /// `01 04 <f32 LE>` -- typed f32.
    F32 { offset: usize, value: f32 },
    /// `C9 <2 bytes BE>` -- u16 big-endian (verified: requisition costs).
    U16Be { offset: usize, value: u16 },
    /// `CF 40 <8 bytes>` -- template GUID ref.
    TemplateGuid { offset: usize, guid: String },
    /// `CF E0 <6 bytes>` -- content GUID ref.
    ContentGuid { offset: usize, guid: String },
    /// Other `CF <subtype> <bytes>` ref forms; subtype recorded for inspection.
    OtherCfRef {
        offset: usize,
        subtype: u8,
        bytes: String,
    },
    /// Unrecognized byte; emitted only when the walker can't proceed.
    Raw { offset: usize, byte: u8 },
}

impl Token {
    fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Token::String { offset, value } => json!({"@": offset, "t": "str", "v": value}),
            Token::TypedString { offset, value } => {
                json!({"@": offset, "t": "str_typed", "v": value})
            }
            Token::ArrayString { offset, idx, value } => {
                json!({"@": offset, "t": "str_arr", "idx": idx, "v": value})
            }
            Token::U8 { offset, value } => json!({"@": offset, "t": "u8", "v": value}),
            Token::U16Le { offset, value } => json!({"@": offset, "t": "u16le", "v": value}),
            Token::F32 { offset, value } => json!({"@": offset, "t": "f32", "v": value}),
            Token::U16Be { offset, value } => json!({"@": offset, "t": "u16be", "v": value}),
            Token::TemplateGuid { offset, guid } => {
                json!({"@": offset, "t": "tmpl_guid", "v": guid})
            }
            Token::ContentGuid { offset, guid } => {
                json!({"@": offset, "t": "content_guid", "v": guid})
            }
            Token::OtherCfRef {
                offset,
                subtype,
                bytes,
            } => {
                json!({"@": offset, "t": "cf_ref", "sub": format!("{:02X}", subtype), "v": bytes})
            }
            Token::Raw { offset, byte } => {
                json!({"@": offset, "t": "raw", "v": format!("{:02X}", byte)})
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Walk a payload emitting tokens. Heuristic-but-defensible: prefers longest
/// match, falls back to single-byte Raw on miss.
fn walk(payload: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < payload.len() {
        let off = i;
        let b = payload[i];

        // 0xD2 0x01 <idx> <len> <ascii>
        if b == 0xD2 && i + 4 <= payload.len() && payload[i + 1] == 0x01 {
            let idx = payload[i + 2];
            let len = payload[i + 3] as usize;
            let start = i + 4;
            if (1..=200).contains(&len) && start + len <= payload.len() {
                let bs = &payload[start..start + len];
                if bs.iter().all(|&c| (32..127).contains(&c)) {
                    out.push(Token::ArrayString {
                        offset: off,
                        idx,
                        value: std::str::from_utf8(bs).unwrap().to_string(),
                    });
                    i = start + len;
                    continue;
                }
            }
        }

        // 0x06 <len> <ascii>
        if b == 0x06 && i + 2 <= payload.len() {
            let len = payload[i + 1] as usize;
            let start = i + 2;
            if (1..=200).contains(&len) && start + len <= payload.len() {
                let bs = &payload[start..start + len];
                if bs.iter().all(|&c| (32..127).contains(&c)) {
                    out.push(Token::String {
                        offset: off,
                        value: std::str::from_utf8(bs).unwrap().to_string(),
                    });
                    i = start + len;
                    continue;
                }
            }
        }

        // 0x01 <type> <bytes>
        if b == 0x01 && i + 1 < payload.len() {
            match payload[i + 1] {
                0x01 if i + 3 <= payload.len() => {
                    out.push(Token::U8 {
                        offset: off,
                        value: payload[i + 2],
                    });
                    i += 3;
                    continue;
                }
                0x02 if i + 4 <= payload.len() => {
                    let v = u16::from_le_bytes([payload[i + 2], payload[i + 3]]);
                    out.push(Token::U16Le {
                        offset: off,
                        value: v,
                    });
                    i += 4;
                    continue;
                }
                0x04 if i + 6 <= payload.len() => {
                    let v = f32::from_le_bytes([
                        payload[i + 2],
                        payload[i + 3],
                        payload[i + 4],
                        payload[i + 5],
                    ]);
                    if v.is_finite() {
                        out.push(Token::F32 {
                            offset: off,
                            value: v,
                        });
                        i += 6;
                        continue;
                    }
                }
                0x06 if i + 3 <= payload.len() => {
                    let len = payload[i + 2] as usize;
                    let start = i + 3;
                    if (1..=200).contains(&len) && start + len <= payload.len() {
                        let bs = &payload[start..start + len];
                        if bs.iter().all(|&c| (32..127).contains(&c)) {
                            out.push(Token::TypedString {
                                offset: off,
                                value: std::str::from_utf8(bs).unwrap().to_string(),
                            });
                            i = start + len;
                            continue;
                        }
                    }
                }
                _ => {}
            }
        }

        // C9 <2 bytes BE> (u16 BE)
        if b == 0xC9 && i + 3 <= payload.len() {
            let v = u16::from_be_bytes([payload[i + 1], payload[i + 2]]);
            out.push(Token::U16Be {
                offset: off,
                value: v,
            });
            i += 3;
            continue;
        }

        // CF 40 <8 bytes>
        if b == 0xCF && i + 10 <= payload.len() && payload[i + 1] == 0x40 {
            out.push(Token::TemplateGuid {
                offset: off,
                guid: hex(&payload[i + 2..i + 10]),
            });
            i += 10;
            continue;
        }

        // CF E0 <6 bytes>
        if b == 0xCF && i + 8 <= payload.len() && payload[i + 1] == 0xE0 {
            out.push(Token::ContentGuid {
                offset: off,
                guid: format!("E0{}", hex(&payload[i + 2..i + 8])),
            });
            i += 8;
            continue;
        }

        // CF <other subtype> <some bytes>: emit a hint, advance 8 bytes total
        // (matches the dominant CF-family slot width). This is heuristic --
        // if the actual width differs, the next token will likely fall back
        // to Raw and the walker recovers within a few bytes.
        if b == 0xCF && i + 8 <= payload.len() {
            let sub = payload[i + 1];
            out.push(Token::OtherCfRef {
                offset: off,
                subtype: sub,
                bytes: hex(&payload[i + 2..i + 8]),
            });
            i += 8;
            continue;
        }

        out.push(Token::Raw {
            offset: off,
            byte: b,
        });
        i += 1;
    }
    out
}

/// Coverage stat: fraction of payload bytes consumed by recognized tokens.
fn coverage(tokens: &[Token], payload_len: usize) -> f64 {
    let raw_count = tokens
        .iter()
        .filter(|t| matches!(t, Token::Raw { .. }))
        .count();
    1.0 - (raw_count as f64 / payload_len as f64)
}

/// Extract structured records from token stream by splitting on a recurring
/// template-GUID anchor. Each anchor occurrence starts a new record.
fn split_records(tokens: &[Token], anchor_guid: &str) -> Vec<Vec<Token>> {
    let mut records: Vec<Vec<Token>> = Vec::new();
    for tok in tokens {
        if let Token::TemplateGuid { guid, .. } = tok {
            if guid == anchor_guid {
                records.push(Vec::new());
            }
        }
        if let Some(last) = records.last_mut() {
            last.push(tok.clone());
        }
    }
    records
}

/// Identify the most-frequent template GUID in a payload -- usually the
/// per-record anchor.
fn dominant_template(tokens: &[Token]) -> Option<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for tok in tokens {
        if let Token::TemplateGuid { guid, .. } = tok {
            *counts.entry(guid.clone()).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(g, _)| g)
}

fn decode_one(fqn: &str, payload: &[u8]) -> serde_json::Value {
    use serde_json::json;
    let tokens = walk(payload);
    let cov = coverage(&tokens, payload.len());
    let dom = dominant_template(&tokens);
    let strings: Vec<&str> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::String { value, .. } | Token::TypedString { value, .. } => Some(value.as_str()),
            Token::ArrayString { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .collect();
    let content_guids: Vec<&str> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::ContentGuid { guid, .. } => Some(guid.as_str()),
            _ => None,
        })
        .collect();

    let records = match dom.as_deref() {
        Some(d) => split_records(&tokens, d),
        None => vec![tokens.clone()],
    };

    json!({
        "fqn": fqn,
        "payload_size": payload.len(),
        "coverage": format!("{:.3}", cov),
        "dominant_template_anchor": dom,
        "string_count": strings.len(),
        "content_guid_count": content_guids.len(),
        "record_count": records.len(),
        "records": records.iter().map(|r| {
            r.iter().map(|t| t.to_json()).collect::<Vec<_>>()
        }).collect::<Vec<_>>(),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut output_dir = PathBuf::from("docs/prototypes-decoded/gsf");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" | "--input" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" | "--hashes" => {
                hash_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "-o" | "--output" => {
                output_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;
    fs::create_dir_all(&output_dir)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut found: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if entry.compressed_size == 0 {
                continue;
            }
            let path = hashes.get(entry.filename_hash);
            let is_bucket = path.map(|p| p.contains("/buckets/")).unwrap_or(false);
            if !is_bucket {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if !pbuk::is_pbuk(&data) {
                continue;
            }
            let objects = match pbuk::parse(&data) {
                Ok(o) => o,
                Err(_) => continue,
            };
            for obj in objects {
                if TARGETS.contains(&obj.fqn.as_str()) && !found.contains_key(&obj.fqn) {
                    found.insert(obj.fqn.clone(), obj.payload.clone());
                }
            }
        }
        if found.len() == TARGETS.len() {
            break;
        }
    }

    println!(
        "Decoded {} of {} target prototypes",
        found.len(),
        TARGETS.len()
    );
    for (fqn, payload) in &found {
        let decoded = decode_one(fqn, payload);
        let out_path = output_dir.join(format!("{fqn}.json"));
        fs::write(&out_path, serde_json::to_string_pretty(&decoded)?)?;
        println!(
            "  {} -> {} ({}B, coverage={}, records={})",
            fqn,
            out_path.display(),
            payload.len(),
            decoded["coverage"].as_str().unwrap_or("?"),
            decoded["record_count"].as_u64().unwrap_or(0)
        );
    }
    for target in TARGETS {
        if !found.contains_key(*target) {
            eprintln!("MISSING: {target}");
        }
    }
    Ok(())
}
