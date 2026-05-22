//! Scan all GSF ability raw STB strings for `<<N>>` and `<<N[...]>>` templates.
//! Joins to spice.sqlite to filter to abl.spvp.* objects only.

use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::stb;
use regex::Regex;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut db_path = PathBuf::from("/Users/seansilvius/swtor/data/spice.sqlite");

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
            "--db" => {
                db_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }

    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;
    let target = "/resources/en-us/str/abl.stb";
    let target_hash = hashes
        .paths_matching(target)
        .into_iter()
        .find(|(_, p)| p == &&target.to_string())
        .map(|(h, _)| h)
        .ok_or_else(|| anyhow::anyhow!("path not in dict"))?;

    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT string_id, fqn FROM objects WHERE fqn LIKE 'abl.spvp.%' AND string_id IS NOT NULL",
    )?;
    let id_to_fqn: HashMap<u32, String> = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let fqn: String = row.get(1)?;
            Ok((id as u32, fqn))
        })?
        .filter_map(|r| r.ok())
        .collect();
    eprintln!("loaded {} abl.spvp.* string_ids from db", id_to_fqn.len());

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let stb_data = (|| -> Result<Vec<u8>> {
        for tor_path in &tor_files {
            let mut archive = Archive::open(tor_path)?;
            let entries: Vec<_> = archive.entries()?.cloned().collect();
            for entry in entries {
                if entry.filename_hash == target_hash {
                    return Ok(archive.read_entry(&entry)?);
                }
            }
        }
        anyhow::bail!("abl.stb not found")
    })()?;
    let stb_file = stb::parse(&stb_data, target)?;
    eprintln!("parsed {} stb entries", stb_file.entries.len());

    let re_simple = Regex::new(r"<<(\d+)>>")?;
    let re_unit = Regex::new(r"<<(\d+)\[([^]]+)\]>>")?;

    let mut family_template_summary: HashMap<String, usize> = HashMap::new();
    let mut sample_per_family: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for entry in &stb_file.entries {
        let Some(fqn) = id_to_fqn.get(&entry.id2) else {
            continue;
        };
        if entry.text.is_empty() {
            continue;
        }
        let has_simple = re_simple.is_match(&entry.text);
        let has_unit = re_unit.is_match(&entry.text);
        if !has_simple && !has_unit {
            continue;
        }
        let family = fqn.split('.').nth(2).unwrap_or("?").to_string();
        *family_template_summary.entry(family.clone()).or_default() += 1;
        sample_per_family
            .entry(family)
            .or_default()
            .push((fqn.clone(), entry.text.clone()));
    }

    println!("FAMILY                  TPL_STRINGS");
    let mut fams: Vec<_> = family_template_summary.iter().collect();
    fams.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
    for (f, c) in fams {
        println!("  {f:<22}  {c}");
    }
    println!();

    for (family, samples) in &sample_per_family {
        println!("=== family: {family} ===");
        for (fqn, text) in samples.iter().take(4) {
            let units: Vec<String> = re_unit
                .captures_iter(text)
                .map(|c| format!("<<{}[{}]>>", &c[1], &c[2]))
                .collect();
            let simple: Vec<String> = re_simple
                .captures_iter(text)
                .map(|c| format!("<<{}>>", &c[1]))
                .collect();
            println!("  {fqn}");
            println!("    text: {text}");
            if !units.is_empty() {
                println!("    unit-tpls: {}", units.join(", "));
            }
            if !simple.is_empty() {
                println!("    simple-tpls: {}", simple.join(", "));
            }
        }
        println!();
    }

    Ok(())
}
