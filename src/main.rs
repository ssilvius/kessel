use anyhow::Result;
use clap::Parser;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod db;
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

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Setup logging
    let level = if args.verbose { Level::DEBUG } else { Level::INFO };
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Kessel spice miner starting...");
    info!("Input directory: {:?}", args.input);
    info!("Output database: {:?}", args.output);

    // Load hash dictionary if provided
    let mut hash_dict = hash::HashDictionary::new();
    let mut bucket_hashes: HashSet<u64> = HashSet::new();

    if let Some(hash_path) = &args.hashes {
        info!("Loading hash dictionary: {:?}", hash_path);
        let count = hash_dict.load(hash_path)?;
        info!("  Loaded {} file hashes", count);

        // Find all bucket file hashes
        for (hash, path) in hash_dict.paths_matching("/buckets/") {
            if path.ends_with(".bkt") {
                bucket_hashes.insert(hash);
            }
        }
        info!("  Found {} bucket files to extract", bucket_hashes.len());
    }

    // Build set of STB file hashes to extract (filtered to the 6 main files)
    let mut stb_hashes: HashSet<u64> = HashSet::new();
    for (hash, path) in hash_dict.paths_matching("/str/") {
        if stb::should_extract_stb(path) {
            stb_hashes.insert(hash);
            tracing::debug!("STB to extract: {} (hash {:016X})", path, hash);
        }
    }
    info!("  Found {} STB files to extract", stb_hashes.len());

    // Initialize database
    let db = db::Database::new(&args.output)?;
    db.init_schema()?;

    // Find all .tor files
    let tor_files: Vec<PathBuf> = std::fs::read_dir(&args.input)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "tor"))
        .collect();

    info!("Found {} .tor files", tor_files.len());

    // Process each archive
    let mut total_bucket_files = 0;
    let mut total_objects = 0;
    let mut total_strings = 0;
    let mut total_stb_files = 0;
    let mut seen_hashes: HashSet<u64> = HashSet::new();

    for tor_path in &tor_files {
        info!("Processing: {:?}", tor_path.file_name().unwrap_or_default());

        match myp::Archive::open(tor_path) {
            Ok(mut archive) => {
                // Collect entries first to release immutable borrow before read_entry
                let entries: Vec<_> = archive.entries()?.cloned().collect();
                let mut pbuk_count = 0;
                let mut bucket_count = 0;

                // Debug: log first few hashes from this archive
                if seen_hashes.is_empty() {
                    for (i, entry) in entries.iter().take(5).enumerate() {
                        tracing::debug!("  Entry {}: hash={:016X}", i, entry.filename_hash);
                        if let Some(path) = hash_dict.get(entry.filename_hash) {
                            tracing::debug!("    -> {}", path);
                        }
                    }
                }

                let mut stb_count = 0;
                let mut string_count = 0;

                for entry in entries {
                    seen_hashes.insert(entry.filename_hash);
                    // Check if this is a known bucket file by hash
                    let is_bucket = bucket_hashes.contains(&entry.filename_hash);
                    // Check if this is an STB file we want to extract
                    let is_stb = stb_hashes.contains(&entry.filename_hash);

                    if is_bucket {
                        tracing::debug!("Reading bucket at pos={}, hdr={}, comp_size={}, uncomp_size={}, compression={}",
                            entry.position, entry.header_size, entry.compressed_size, entry.uncompressed_size, entry.compression);
                    }

                    match archive.read_entry(&entry) {
                        Ok(data) => {
                        // Process STB files first (before bucket check)
                        if is_stb {
                            if let Some(path) = hash_dict.get(entry.filename_hash) {
                                tracing::debug!("  STB: {} ({} bytes)", path, data.len());
                                match stb::parse(&data, path) {
                                    Ok(stb_file) => {
                                        stb_count += 1;
                                        for entry in &stb_file.entries {
                                            // Build FQN for this string entry
                                            // TODO: We need to figure out how STB entries map to FQNs
                                            // For now, use the file's FQN prefix + id1.id2
                                            let string_fqn = format!("{}.{}.{}", stb_file.fqn_prefix, entry.id1, entry.id2);
                                            if let Err(e) = db.insert_string(&string_fqn, &stb_file.locale, entry) {
                                                tracing::warn!("Failed to insert string: {}", e);
                                            } else {
                                                string_count += 1;
                                            }
                                        }
                                        tracing::debug!("  Parsed {} strings from {}", stb_file.entries.len(), path);
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to parse STB {}: {}", path, e);
                                    }
                                }
                            }
                        }
                        // Process bucket files (PBUK format)
                        else if is_bucket {
                            bucket_count += 1;
                            if let Some(path) = hash_dict.get(entry.filename_hash) {
                                tracing::debug!("  Bucket: {} ({} bytes)", path, data.len());
                            }

                            // Debug: log first bytes of decompressed bucket file
                            if bucket_count <= 3 && data.len() >= 32 {
                                let magic: String = data[0..4].iter()
                                    .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                                    .collect();
                                tracing::info!("  Bucket magic: {} (size: {} bytes)", magic, data.len());
                                tracing::info!("  First 32 bytes: {:02X?}", &data[0..32]);
                                tracing::info!("  is_pbuk={}, is_dblb={}", pbuk::is_pbuk(&data), pbuk::is_dblb(&data));
                            }

                            // Try PBUK first, then DBLB, then raw
                            if pbuk::is_pbuk(&data) {
                                if let Ok(count) = process_pbuk(&data, &db) {
                                    total_objects += count;
                                }
                            } else if pbuk::is_dblb(&data) {
                                if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                                    for obj in objects {
                                        let game_obj = schema::GameObject::from_gom(&obj);
                                        if should_extract_object(&game_obj.fqn) && !game_obj.fqn.is_empty() {
                                            db.insert_object(&game_obj)?;
                                            total_objects += 1;
                                        }
                                    }
                                }
                            } else {
                                tracing::debug!("  Bucket not PBUK or DBLB format");
                            }
                        } else if pbuk::is_pbuk(&data) {
                            pbuk_count += 1;

                            if let Ok(count) = process_pbuk(&data, &db) {
                                total_objects += count;
                            }
                        } else if pbuk::is_dblb(&data) {
                            // Direct DBLB without PBUK wrapper
                            if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                                for obj in objects {
                                    let game_obj = schema::GameObject::from_gom(&obj);
                                    if should_extract_object(&game_obj.fqn) && !game_obj.fqn.is_empty() {
                                        db.insert_object(&game_obj)?;
                                        total_objects += 1;
                                    }
                                }
                            }
                        }
                        }
                        Err(e) => {
                            if is_bucket {
                                tracing::warn!("Failed to read bucket: {}", e);
                            }
                        }
                    }
                }

                total_bucket_files += bucket_count;
                total_stb_files += stb_count;
                total_strings += string_count;
                if pbuk_count > 0 || bucket_count > 0 || stb_count > 0 {
                    tracing::info!("  Found {} bucket files, {} PBUK files, {} STB files ({} strings)",
                        bucket_count, pbuk_count, stb_count, string_count);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to open {:?}: {}", tor_path, e);
            }
        }
    }

    info!("Processed {} bucket files, extracted {} objects", total_bucket_files, total_objects);
    info!("Processed {} STB files, extracted {} strings", total_stb_files, total_strings);
    info!("Scanned {} unique file hashes across all archives", seen_hashes.len());

    // Debug: show how many bucket hashes were found in archives
    let found_buckets: Vec<_> = bucket_hashes.iter().filter(|h| seen_hashes.contains(h)).collect();
    info!("Bucket hashes found in archives: {}/{}", found_buckets.len(), bucket_hashes.len());

    // Sample some bucket hashes to see format
    if !found_buckets.is_empty() {
        for hash in found_buckets.iter().take(3) {
            if let Some(path) = hash_dict.get(**hash) {
                info!("  Found bucket: {} (hash {:016X})", path, hash);
            }
        }
    } else {
        // Show sample bucket hashes to debug
        info!("Sample bucket hashes from dictionary:");
        for (hash, path) in hash_dict.paths_matching("/buckets/").iter().take(3) {
            info!("  {:016X} -> {}", hash, path);
        }
        info!("Sample file hashes from archives:");
        for hash in seen_hashes.iter().take(3) {
            if let Some(path) = hash_dict.get(*hash) {
                info!("  {:016X} -> {}", hash, path);
            } else {
                info!("  {:016X} -> (unknown)", hash);
            }
        }
    }

    let stats = db.stats()?;
    info!("Extraction complete!");
    info!("  Quests: {}", stats.quests);
    info!("  Abilities: {}", stats.abilities);
    info!("  Items: {}", stats.items);
    info!("  NPCs: {}", stats.npcs);
    info!("  Strings: {}", stats.strings);

    Ok(())
}

/// Check if a GOM object should be extracted based on FQN prefix
///
/// We keep: abl, itm, npc, schem, qst, cdx, ach, mpn
/// We skip: spn, hyd, plc, epp, cnd, npp, dyn, enc, stg, apn, etc.
///
/// Quality filters:
/// - Skip test/debug/deprecated data
/// - Skip internal NPC/encounter-specific abilities
/// - Skip versioned duplicates (only keep base FQN)
fn should_extract_object(fqn: &str) -> bool {
    // Skip versioned duplicates: "abl.foo.bar/17/5" -> only keep base "abl.foo.bar"
    if fqn.contains('/') {
        return false;
    }

    let prefix = match fqn.find('.') {
        Some(pos) => &fqn[..pos],
        None => fqn,
    };

    // First check: must be a known prefix type
    if !matches!(
        prefix,
        "abl" | "itm" | "npc" | "schem" | "qst" | "cdx" | "ach" | "mpn"
    ) {
        return false;
    }

    // Quality filters: skip junk data
    let parts: Vec<&str> = fqn.split('.').collect();

    // Skip test, debug, deprecated content (anywhere in FQN)
    for part in &parts {
        if matches!(
            *part,
            "test" | "debug" | "deprecated" | "obsolete" | "old" | "qa" | "dev"
        ) {
            return false;
        }
    }

    // Also check the full FQN for test-related substrings
    if fqn.contains(".test_") || fqn.contains("_test.") || fqn.contains(".debug_") {
        return false;
    }

    // For abilities: skip internal/encounter-specific ones
    if prefix == "abl" && parts.len() >= 2 {
        let second = parts[1];
        // Skip: NPC abilities, encounter mechanics, internal systems
        if matches!(
            second,
            "npc"           // NPC-only abilities
            | "qtr"         // Raid/operation encounter abilities
            | "operation"   // Operation boss mechanics
            | "flashpoint"  // Flashpoint encounter abilities
            | "dynamic_events" // World event mechanics
            | "world_design"   // Environment/world abilities
            | "placeables"     // Object abilities
            | "ballistics"     // Physics test abilities
            | "state"          // Internal state abilities
            | "creature"       // Creature-only abilities
            | "exp"            // Expansion quest mechanics
            | "quest"          // Quest-specific abilities
            | "daily_area"     // Daily area mechanics
            | "alliance"       // Alliance system abilities
            | "command"        // Command XP consumables
            | "conquest"       // Conquest internal mechanics
            | "e3"             // E3 demo/event abilities
            | "event"          // Event NPC mechanics
            | "galactic_seasons" // Season reward mechanics
            | "gld"            // Guild internal mechanics
            | "itm"            // Item use abilities (internal)
            | "mtx"            // Cartel Market unlock abilities
            | "player"         // Internal player mount/emote abilities
            | "pvp"            // Warzone internal mechanics
            | "reputation"     // Reputation system mechanics
            | "stronghold"     // Stronghold decoration abilities
            | "strongholds"    // Stronghold decoration abilities
            | "ventures"       // GTN/Trade abilities
            | "creature_default" // Default creature abilities
            | "droid"          // Droid internal abilities
            | "flurry"         // Internal combat flurry system
            | "generic"        // Generic placeholder abilities
        ) {
            return false;
        }
    }

    // For items: skip internal/NPC variants
    if prefix == "itm" && parts.len() >= 2 {
        let second = parts[1];
        // Skip: NPC gear variants, internal loot tables, condition checks
        if matches!(
            second,
            "npc"           // NPC-only gear (asset variants)
            | "loot"        // Loot table definitions
            | "has_item"    // Item ownership checks (conditions)
            | "slot_is_lowest"  // Slot rating checks
            | "slot_is_rating"  // Slot rating checks
            | "irating"     // Item rating threshold checks
            | "ach"         // Achievement unlock items
            | "codex"       // Codex unlock items
            | "mercury"     // Internal currency items
            | "location"    // Location unlock items
        ) {
            return false;
        }
    }

    // For NPCs: skip internal templates and blueprints
    if prefix == "npc" && parts.len() >= 2 {
        let second = parts[1];
        // Skip: blueprints, generic templates, combat spawns
        if matches!(
            second,
            "blueprints"       // NPC template definitions
            | "ability"        // NPC ability containers
            | "combat"         // Combat spawn definitions
            | "cinematic_extras" // Cinematic background NPCs
            | "heavy_weight_cos" // Internal costume variants
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
        // Convert GomObject to GameObject for storage
        let game_obj = schema::GameObject::from_gom(&obj);
        // Apply filter: only keep relevant object types
        if should_extract_object(&game_obj.fqn) && !game_obj.fqn.is_empty() {
            db.insert_object(&game_obj)?;
            count += 1;
        }
    }

    Ok(count)
}
