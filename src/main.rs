use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod myp;
mod pbuk;
mod xml_parser;
mod schema;
mod db;

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
    for tor_path in &tor_files {
        info!("Processing: {:?}", tor_path.file_name().unwrap_or_default());

        match myp::Archive::open(tor_path) {
            Ok(mut archive) => {
                // Collect entries first to release immutable borrow before read_entry
                let entries: Vec<_> = archive.entries()?.cloned().collect();
                for entry in entries {
                    // Check if this is a PBUK/DBLB file (GOM data)
                    if let Ok(data) = archive.read_entry(&entry) {
                        if pbuk::is_pbuk(&data) {
                            process_pbuk(&data, &db)?;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to open {:?}: {}", tor_path, e);
            }
        }
    }

    let stats = db.stats()?;
    info!("Extraction complete!");
    info!("  Quests: {}", stats.quests);
    info!("  Abilities: {}", stats.abilities);
    info!("  Items: {}", stats.items);
    info!("  NPCs: {}", stats.npcs);

    Ok(())
}

fn process_pbuk(data: &[u8], db: &db::Database) -> Result<()> {
    let chunks = pbuk::parse(data)?;

    for chunk in chunks {
        if let Some(xml_data) = chunk.decompress()? {
            if let Ok(obj) = xml_parser::parse(&xml_data) {
                db.insert_object(&obj)?;
            }
        }
    }

    Ok(())
}
