use anyhow::Result;
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod db;
mod dds;
mod hash;
mod myp;
mod pbuk;
mod schema;
mod stb;

#[derive(Parser, Debug)]
#[command(name = "kessel")]
#[command(about = "SWTOR data miner - extracts game objects from .tor archives")]
struct Args {
    /// Directory containing .tor files
    #[arg(short, long)]
    input: PathBuf,

    /// Output SQLite database path
    #[arg(short, long, default_value = "raw.sqlite")]
    output: PathBuf,

    /// Hash dictionary file (hashes_filename.txt from Jedipedia)
    #[arg(short = 'H', long)]
    hashes: Option<PathBuf>,

    /// Only process specific file types (quest, ability, item, npc)
    #[arg(short, long)]
    filter: Option<Vec<String>>,

    /// Extract ability icons to WebP format
    #[arg(long)]
    icons: bool,

    /// Output directory for icons (default: ./icons)
    #[arg(long, default_value = "icons")]
    icons_output: PathBuf,

    /// Verbose output (show debug info)
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Load hash dictionary if provided
    let mut hash_dict = hash::HashDictionary::new();
    let mut bucket_hashes: HashSet<u64> = HashSet::new();

    if let Some(hash_path) = &args.hashes {
        hash_dict.load(hash_path)?;

        // Find all bucket file hashes
        for (hash, path) in hash_dict.paths_matching("/buckets/") {
            if path.ends_with(".bkt") {
                bucket_hashes.insert(hash);
            }
        }
    }

    // Build set of STB file hashes to extract
    let mut stb_hashes: HashSet<u64> = HashSet::new();
    for (hash, path) in hash_dict.paths_matching("/str/") {
        if stb::should_extract_stb(path) {
            stb_hashes.insert(hash);
        }
    }

    // Build set of icon file hashes to extract
    let mut icon_hashes: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
    if args.icons {
        for (hash, path) in hash_dict.paths_matching("/gfx/icons/") {
            if path.ends_with(".dds") {
                icon_hashes.insert(hash, path.to_string());
            }
        }
        // Create icons output directory
        std::fs::create_dir_all(&args.icons_output)?;
    }

    // Initialize database
    let db = db::Database::new(&args.output)?;
    db.init_schema()?;

    // Find all .tor files
    let mut tor_files: Vec<PathBuf> = std::fs::read_dir(&args.input)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "tor"))
        .collect();
    tor_files.sort();

    // Setup progress bars
    let multi = MultiProgress::new();
    let main_style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
        .unwrap()
        .progress_chars("=>-");

    let entry_style = ProgressStyle::default_bar()
        .template("  {spinner:.yellow} {msg} [{bar:30.yellow/blue}] {pos}/{len}")
        .unwrap()
        .progress_chars("=>-");

    let main_pb = multi.add(ProgressBar::new(tor_files.len() as u64));
    main_pb.set_style(main_style);
    main_pb.set_message("archives");

    // Counters
    let mut total_objects = 0usize;
    let mut total_strings = 0usize;
    let mut total_icons = 0usize;
    let mut seen_hashes: HashSet<u64> = HashSet::new();
    let mut seen_icon_content: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for tor_path in &tor_files {
        let filename = tor_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        main_pb.set_message(filename.clone());

        match myp::Archive::open(tor_path) {
            Ok(mut archive) => {
                let entries: Vec<_> = match archive.entries() {
                    Ok(iter) => iter.cloned().collect(),
                    Err(_) => {
                        main_pb.inc(1);
                        continue;
                    }
                };

                let entry_count = entries.len();
                let archive_start = Instant::now();
                let mut entry_pb: Option<ProgressBar> = None;
                let mut last_check = Instant::now();

                for (i, entry) in entries.iter().enumerate() {
                    seen_hashes.insert(entry.filename_hash);

                    // Show entry progress bar if archive takes >20s
                    if entry_pb.is_none() && archive_start.elapsed() > Duration::from_secs(20) {
                        let pb = multi.insert_after(&main_pb, ProgressBar::new(entry_count as u64));
                        pb.set_style(entry_style.clone());
                        pb.set_position(i as u64);
                        pb.set_message(filename.clone());
                        entry_pb = Some(pb);
                    }

                    // Update entry progress every 100ms
                    if let Some(ref pb) = entry_pb {
                        if last_check.elapsed() > Duration::from_millis(100) {
                            pb.set_position(i as u64);
                            last_check = Instant::now();
                        }
                    }

                    let is_bucket = bucket_hashes.contains(&entry.filename_hash);
                    let is_stb = stb_hashes.contains(&entry.filename_hash);

                    let data = match archive.read_entry(entry) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    // Process STB files
                    if is_stb {
                        if let Some(path) = hash_dict.get(entry.filename_hash) {
                            if let Ok(stb_file) = stb::parse(&data, path) {
                                for stb_entry in &stb_file.entries {
                                    let string_fqn = format!(
                                        "{}.{}.{}",
                                        stb_file.fqn_prefix, stb_entry.id1, stb_entry.id2
                                    );
                                    if db
                                        .insert_string(&string_fqn, &stb_file.locale, stb_entry)
                                        .is_ok()
                                    {
                                        total_strings += 1;
                                    }
                                }
                            }
                        }
                    }
                    // Process bucket files (PBUK format)
                    else if is_bucket {
                        if pbuk::is_pbuk(&data) {
                            if let Ok(count) = process_pbuk(&data, &db) {
                                total_objects += count;
                            }
                        } else if pbuk::is_dblb(&data) {
                            if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                                for obj in objects {
                                    let game_obj = schema::GameObject::from_gom(&obj);
                                    if should_extract_object(&game_obj.fqn)
                                        && !game_obj.fqn.is_empty()
                                    {
                                        if db.insert_object(&game_obj).is_ok() {
                                            total_objects += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Process loose PBUK/DBLB files
                    else if pbuk::is_pbuk(&data) {
                        if let Ok(count) = process_pbuk(&data, &db) {
                            total_objects += count;
                        }
                    } else if pbuk::is_dblb(&data) {
                        if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                            for obj in objects {
                                let game_obj = schema::GameObject::from_gom(&obj);
                                if should_extract_object(&game_obj.fqn) && !game_obj.fqn.is_empty()
                                {
                                    if db.insert_object(&game_obj).is_ok() {
                                        total_objects += 1;
                                    }
                                }
                            }
                        }
                    }

                    // Process icon files (DDS -> WebP)
                    if let Some(icon_path) = icon_hashes.get(&entry.filename_hash) {
                        if dds::is_dds(&data) {
                            match dds::convert_to_webp(&data, icon_path) {
                                Ok(icon) => {
                                    // Deduplicate by content hash
                                    if !seen_icon_content.contains_key(&icon.content_hash) {
                                        seen_icon_content.insert(icon.content_hash.clone(), icon.icon_name.clone());
                                        if dds::save_icon(&icon, &args.icons_output).is_ok() {
                                            total_icons += 1;
                                        }
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                    }
                }

                // Clear entry progress bar
                if let Some(pb) = entry_pb {
                    pb.finish_and_clear();
                }
            }
            Err(_) => {}
        }

        main_pb.inc(1);
    }

    main_pb.finish_and_clear();

    // Print summary
    let stats = db.stats()?;
    println!("\nExtraction complete!");
    println!("  Archives: {}", tor_files.len());
    println!("  File hashes scanned: {}", seen_hashes.len());
    println!();
    println!("  Objects: {}", total_objects);
    println!("    Quests: {}", stats.quests);
    println!("    Abilities: {}", stats.abilities);
    println!("    Items: {}", stats.items);
    println!("    NPCs: {}", stats.npcs);
    println!();
    println!("  Strings: {}", stats.strings);
    if args.icons {
        println!();
        println!("  Icons: {} (deduplicated)", total_icons);
        println!("    Output: {}", args.icons_output.display());
    }

    Ok(())
}

/// Check if a GOM object should be extracted based on FQN prefix
fn should_extract_object(fqn: &str) -> bool {
    // Skip versioned duplicates: "abl.foo.bar/17/5" -> only keep base "abl.foo.bar"
    if fqn.contains('/') {
        return false;
    }

    let prefix = match fqn.find('.') {
        Some(pos) => &fqn[..pos],
        None => fqn,
    };

    // Must be a known prefix type
    if !matches!(
        prefix,
        "abl" | "tal" | "itm" | "npc" | "schem" | "qst" | "cdx" | "ach" | "mpn"
        | "pkg" | "loot" | "rew" | "cnv"
    ) {
        return false;
    }

    let parts: Vec<&str> = fqn.split('.').collect();

    // Skip test, debug, deprecated content
    for part in &parts {
        if matches!(
            *part,
            "test" | "debug" | "deprecated" | "obsolete" | "old" | "qa" | "dev"
        ) {
            return false;
        }
    }

    if fqn.contains(".test_") || fqn.contains("_test.") || fqn.contains(".debug_") {
        return false;
    }

    // Skip internal abilities
    if prefix == "abl" && parts.len() >= 2 {
        let second = parts[1];
        if matches!(
            second,
            "npc"
                | "qtr"
                | "operation"
                | "flashpoint"
                | "dynamic_events"
                | "world_design"
                | "placeables"
                | "ballistics"
                | "state"
                | "creature"
                | "exp"
                | "quest"
                | "daily_area"
                | "alliance"
                | "command"
                | "conquest"
                | "e3"
                | "event"
                | "galactic_seasons"
                | "gld"
                | "itm"
                | "mtx"
                | "player"
                | "pvp"
                | "reputation"
                | "stronghold"
                | "strongholds"
                | "ventures"
                | "creature_default"
                | "droid"
                | "flurry"
                | "generic"
        ) {
            return false;
        }
    }

    // Skip internal items
    if prefix == "itm" && parts.len() >= 2 {
        let second = parts[1];
        if matches!(
            second,
            "npc"
                | "loot"
                | "has_item"
                | "slot_is_lowest"
                | "slot_is_rating"
                | "irating"
                | "ach"
                | "codex"
                | "mercury"
                | "location"
        ) {
            return false;
        }
    }

    // Skip internal NPCs
    if prefix == "npc" && parts.len() >= 2 {
        let second = parts[1];
        if matches!(
            second,
            "blueprints" | "ability" | "combat" | "cinematic_extras" | "heavy_weight_cos"
        ) {
            return false;
        }
    }

    true
}

fn process_pbuk(data: &[u8], db: &db::Database) -> Result<usize> {
    let objects = pbuk::parse(data)?;
    let mut count = 0;

    for obj in objects {
        let game_obj = schema::GameObject::from_gom(&obj);
        if should_extract_object(&game_obj.fqn) && !game_obj.fqn.is_empty() {
            db.insert_object(&game_obj)?;
            count += 1;
        }
    }

    Ok(count)
}
