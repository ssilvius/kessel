//! Scan raw .tor PBUK rows for the 3 specific apc.* FQNs that are missing
//! from kessel's current extraction (causing discipline icon/mod_tree gaps
//! for Shadow Kinetic Combat, Shadow Serenity, Commando Gunnery).
//!
//! Answers: are these FQNs in the source archive but dropped during
//! extraction (kessel bug), or absent from the source entirely (Bioware
//! data gap that needs a fallback)?
//!
//! Usage: scan_missing_apc -i ~/swtor/assets -H ~/swtor/data/hashes_filename.txt

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::pbuk;
use std::path::PathBuf;

const TARGETS: &[&str] = &[
    "apc.jedi_consular.shadow.combat",
    "apc.jedi_consular.shadow.combat_mods", // present (sanity)
    "apc.jedi_consular.shadow.serenity",    // present (sanity)
    "apc.jedi_consular.shadow.serenity_mods",
    "apc.trooper.commando.gunnery", // present (sanity)
    "apc.trooper.commando.gunnery_mods",
    // Also probe for kinetic_combat variant naming
    "apc.jedi_consular.shadow.kinetic_combat",
    "apc.jedi_consular.shadow.kinetic_combat_mods",
];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
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
            _ => i += 1,
        }
    }
    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tor"))
        .collect();
    eprintln!(
        "Scanning {} .tor files for {} target FQNs",
        tor_files.len(),
        TARGETS.len()
    );

    let mut hits: std::collections::HashMap<&'static str, Vec<String>> =
        std::collections::HashMap::new();
    for t in TARGETS {
        hits.insert(*t, Vec::new());
    }

    for tor_path in &tor_files {
        let Ok(mut archive) = Archive::open(tor_path) else {
            continue;
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(it) => it.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            let path = match hashes.get(entry.filename_hash) {
                Some(p) => p.clone(),
                None => continue,
            };
            if !path.contains("/buckets/") || !path.ends_with(".bkt") {
                continue;
            }
            let Ok(data) = archive.read_entry(entry) else {
                continue;
            };
            let Ok(objects) = pbuk::parse(&data) else {
                continue;
            };
            for obj in &objects {
                for t in TARGETS {
                    if obj.fqn == *t || obj.fqn.starts_with(&format!("{t}/")) {
                        hits.get_mut(*t).unwrap().push(format!(
                            "{}/{}",
                            tor_path.file_name().unwrap().to_string_lossy(),
                            obj.fqn
                        ));
                    }
                }
            }
        }
    }

    println!("\n--- Scan results ---");
    for t in TARGETS {
        let h = hits.get(t).unwrap();
        println!("  {} hit(s) for {}", h.len(), t);
        for line in h.iter().take(3) {
            println!("    {line}");
        }
    }
    Ok(())
}
