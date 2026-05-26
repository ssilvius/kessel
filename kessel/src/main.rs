use anyhow::Result;
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod buckets_info;
mod db;
mod dds;
mod gifts;
mod gom_schema;
mod grammar;
mod gsf_stat_dictionary;
mod hash;
mod icon_overrides;
mod myp;
mod pbuk;
mod prototypes_info;
mod quest;
mod schema;
mod scpt;
mod stb;
mod unknowns;
mod xml_utf16;

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

    /// Output file for unknown patterns (JSONL format)
    #[arg(long)]
    unknowns: Option<PathBuf>,

    /// Extract all objects without content filtering (filter in ETL instead)
    /// Only excludes versioned duplicates and test/debug content
    #[arg(long)]
    unfiltered: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize unknowns tracker
    let unknowns_writer = if let Some(ref unknowns_path) = args.unknowns {
        unknowns::UnknownsWriter::new(unknowns_path)?
    } else {
        unknowns::UnknownsWriter::disabled()
    };

    // Load embedded grammar rules (compiled into binary)
    let grammar = match grammar::Grammar::from_embedded() {
        Ok(g) => Some(std::sync::Arc::new(g)),
        Err(e) => {
            eprintln!("Warning: Failed to load grammar rules: {}", e);
            None
        }
    };

    // Load embedded icon overrides (compiled into binary)
    let icon_overrides = match icon_overrides::IconOverrides::from_embedded() {
        Ok(o) => Some(o),
        Err(e) => {
            eprintln!("Warning: Failed to load icon overrides: {}", e);
            None
        }
    };

    // Load hash dictionary (auto-download from Jedipedia if not provided)
    let mut hash_dict = hash::HashDictionary::new();
    let mut bucket_hashes: HashSet<u64> = HashSet::new();

    let hash_path = resolve_hashes_path(&args)?;
    if let Some(hash_path) = &hash_path {
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

    // Initialize database with optional grammar rules
    let db = db::Database::with_grammar(&args.output, grammar)?;
    db.init_schema()?;

    // Find all .tor files
    let mut tor_files: Vec<PathBuf> = std::fs::read_dir(&args.input)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "tor"))
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
    let mut total_icons = 0usize;
    let mut seen_hashes: HashSet<u64> = HashSet::new();
    // Per-FQN best-variant score so far. Many FQNs appear multiple times
    // across archives -- some as canonical objects with full payload, some as
    // stub references. Picking first-seen (the prior HashSet behaviour)
    // produced 77% NULL string_id and 80% NULL icon_name for abilities because
    // stubs frequently came first in iteration order. Scoring prefers
    // candidates that resolved a string_id, then icon_name, then larger
    // payloads. Better candidates are still inserted; inferior ones are
    // skipped. A SQL dedup pass after extraction collapses any remaining
    // multi-GUID FQNs to the best row.
    let mut versioned_seen: HashMap<String, u64> = HashMap::new();

    // Buffer icons until objects are processed (need icon_name → game_id mapping)
    let mut pending_icons: Vec<(Vec<u8>, String)> = Vec::new(); // (dds_data, icon_path)

    for tor_path in &tor_files {
        let filename = tor_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        main_pb.set_message(filename.clone());

        if let Ok(mut archive) = myp::Archive::open(tor_path) {
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
                                let _ = db.insert_string(&string_fqn, &stb_file.locale, stb_entry);
                            }
                        }
                    }
                }
                // Process bucket files (PBUK format)
                else if is_bucket {
                    if pbuk::is_pbuk(&data) {
                        if let Ok(count) = process_pbuk(
                            &data,
                            &db,
                            args.unfiltered,
                            icon_overrides.as_ref(),
                            &mut versioned_seen,
                        ) {
                            total_objects += count;
                        }
                    } else if pbuk::is_dblb(&data) {
                        if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                            for mut obj in objects {
                                let Some(fqn) = normalize_fqn(&obj.fqn) else {
                                    continue;
                                };
                                obj.fqn = fqn.clone();
                                if is_singleton_fqn(&fqn) {
                                    if db.insert_singleton(&obj).is_ok() {
                                        total_objects += 1;
                                    }
                                    continue;
                                }
                                let game_obj = schema::GameObject::from_gom_with_overrides(
                                    &obj,
                                    icon_overrides.as_ref(),
                                );
                                if !accept_variant(&mut versioned_seen, &fqn, &game_obj) {
                                    continue;
                                }
                                if should_extract_object(&game_obj.fqn, args.unfiltered)
                                    && !game_obj.fqn.is_empty()
                                    && db.insert_object(&game_obj).is_ok()
                                {
                                    total_objects += 1;
                                }
                            }
                        }
                    }
                }
                // Process loose PBUK/DBLB files
                else if pbuk::is_pbuk(&data) {
                    if let Ok(count) = process_pbuk(
                        &data,
                        &db,
                        args.unfiltered,
                        icon_overrides.as_ref(),
                        &mut versioned_seen,
                    ) {
                        total_objects += count;
                    }
                } else if pbuk::is_dblb(&data) {
                    if let Ok(objects) = pbuk::parse_dblb_direct(&data) {
                        for mut obj in objects {
                            let Some(fqn) = normalize_fqn(&obj.fqn) else {
                                continue;
                            };
                            obj.fqn = fqn.clone();
                            if is_singleton_fqn(&fqn) {
                                if db.insert_singleton(&obj).is_ok() {
                                    total_objects += 1;
                                }
                                continue;
                            }
                            let game_obj = schema::GameObject::from_gom_with_overrides(
                                &obj,
                                icon_overrides.as_ref(),
                            );
                            if !accept_variant(&mut versioned_seen, &fqn, &game_obj) {
                                continue;
                            }
                            if should_extract_object(&game_obj.fqn, args.unfiltered)
                                && !game_obj.fqn.is_empty()
                                && db.insert_object(&game_obj).is_ok()
                            {
                                total_objects += 1;
                            }
                        }
                    }
                }

                // Buffer icon files for processing after objects (need icon_name → game_id mapping)
                if let Some(icon_path) = icon_hashes.get(&entry.filename_hash) {
                    if dds::is_dds(&data) {
                        pending_icons.push((data.clone(), icon_path.clone()));
                    }
                }
            }

            // Clear entry progress bar
            if let Some(pb) = entry_pb {
                pb.finish_and_clear();
            }
        }

        main_pb.inc(1);
    }

    main_pb.finish_and_clear();

    // Process buffered icons now that we have the icon_name → (game_id, kind) mapping
    if args.icons && !pending_icons.is_empty() {
        println!("\nProcessing {} icons...", pending_icons.len());

        // Get mapping: icon_name (SWTOR's) → (game_id, kind)
        let mut icon_mapping = db.get_icon_mapping()?;
        println!("  Icon mapping entries: {}", icon_mapping.len());

        // Merge fallback mappings for objects with NULL icon_name but known FQN patterns
        let fallbacks = db.get_fqn_fallback_icons()?;
        let fallback_count = fallbacks.len();
        for (icon_name, objects) in fallbacks {
            icon_mapping.entry(icon_name).or_default().extend(objects);
        }
        if fallback_count > 0 {
            println!("  FQN-derived fallback icons: {}", fallback_count);
        }

        let mut seen_content: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut unmapped_icons = 0usize;

        for (dds_data, icon_path) in &pending_icons {
            if let Ok(mut icon) = dds::convert_to_webp(dds_data, icon_path) {
                // Deduplicate by content hash
                if seen_content.contains_key(&icon.content_hash) {
                    continue;
                }

                // Extract icon_name from path: "/resources/gfx/icons/abl_foo.dds" → "abl_foo"
                // Lowercase for case-insensitive matching with DB icon_names
                let icon_name = icon_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(icon_path)
                    .trim_end_matches(".dds")
                    .to_lowercase();

                // Skip if we already processed this exact content
                if seen_content.contains_key(&icon.content_hash) {
                    continue;
                }
                seen_content.insert(icon.content_hash.clone(), icon_name.clone());

                // Look up all objects that reference this icon
                if let Some(objects) = icon_mapping.get(&icon_name) {
                    // Save icon for ALL objects that reference it (handles shared icons)
                    for (game_id, kind) in objects {
                        let subdir = match kind.as_str() {
                            "Ability" => "abilities",
                            "Item" => "items",
                            "Npc" => "npcs",
                            "Quest" => "quests",
                            "Achievement" => "achievements",
                            "Codex" => "codex",
                            "Schematic" => "schematics",
                            "Talent" => "talents",
                            _ => "misc",
                        };
                        let output_dir = args.icons_output.join(subdir);
                        icon.icon_id = game_id.clone();
                        if dds::save_icon(&icon, &output_dir).is_ok() {
                            total_icons += 1;
                        }
                    }
                } else {
                    // Unmapped icon - save to misc with original hash
                    unmapped_icons += 1;
                    unknowns_writer.record(unknowns::Unknown::UnmappedIcon {
                        icon_name: icon_name.to_string(),
                        source_file: icon_path.clone(),
                    });
                    let output_dir = args.icons_output.join("misc");
                    if dds::save_icon(&icon, &output_dir).is_ok() {
                        total_icons += 1;
                    }
                }
            }
        }

        if unmapped_icons > 0 {
            println!("  Unmapped icons (fallback naming): {}", unmapped_icons);
        }
    }

    // First post-extraction pass: pick the canonical row per FQN. Same quality
    // heuristic as the old DELETE-based dedup, but lossless -- inferior
    // variants stay in the table with `is_canonical = 0` so delta tooling can
    // read them. Consumers filter `WHERE is_canonical = 1` for the canonical
    // set.
    let demoted = db.mark_canonical_by_fqn()?;
    if demoted > 0 {
        println!(
            "  Demoted {} inferior FQN variants (is_canonical=0)",
            demoted
        );
    }

    // Heuristic: backfill string_id for canonical abl.* rows whose payload
    // didn't carry a string-table marker. Looks up STB strings by display
    // name derived from the FQN's last segment, only commits when there's
    // exactly one unused candidate that has both a name (id1=0) and a
    // description (id1=1).
    let backfilled = db.backfill_missing_string_ids()?;
    if backfilled > 0 {
        println!(
            "  Backfilled {} string_id linkages by display-name match",
            backfilled
        );
    }

    // Second pass: populate quest tables from extracted objects
    let quest_count = db.populate_quest_tables()?;

    // Schema-aware quest typed columns (#129 foundation): activity_type,
    // difficulty, rewards_visibility, episode_season, level. Marker-presence
    // pass; real value decode lands in a follow-on PR.
    let quest_typed = db.populate_quest_details_typed()?;
    println!("  Quest details typed: {} rows updated", quest_typed);

    // Schema-aware quest objectives (#130 foundation): marker-presence pass
    // recording quests that emit QuestObjective class_ref markers. Per-objective
    // field decode lands in a follow-on PR.
    let objectives_count = db.populate_quest_objectives()?;
    println!("  Quest objectives recorded: {}", objectives_count);

    // Expand quest_prerequisites flag-graph (#131, closes #67) -- widens the
    // payload-string prefix whitelist from "has_" only to the full SWTOR
    // flag family (qstrew_, qstv_, cflag_, glob_, cdx_, ach_completed_,
    // completed_).
    let prereq_count = db.populate_quest_prerequisites_graph()?;
    println!("  Quest prereq edges: {}", prereq_count);

    // Item classification from FQN (#59): slot, rating, rarity, source, etc.
    let item_count = db.populate_item_tables()?;
    println!("  Items classified: {}", item_count);

    // Item sets (#105): membership and set display name from itm.setbonus.* FQNs.
    let (sets_count, set_members_count) = db.populate_item_sets()?;
    println!(
        "  Item sets: {} sets, {} members",
        sets_count, set_members_count
    );

    // (Quest chain population removed in #19: PR #11's 0xCF GUID-ref
    // hypothesis produced zero rows on real data.)

    // Third pass: resolve a:enc.* refs in quest payloads to npc.* via encounter payloads
    db.populate_quest_npcs()?;

    // Third-pass complement (#132 / closes #48 #49): scan quest payload strings
    // for direct npc.* refs that the enc/spn graph misses. Picks up planetary
    // side-quest givers and interact targets named inline.
    let direct_npc_count = db.populate_quest_npcs_direct()?;
    println!("  Direct quest->npc edges: {}", direct_npc_count);

    // Fourth pass: extract quest_reward_* variable names from quest payloads
    db.populate_quest_rewards()?;

    // Fifth pass: extract spawn runtime IDs from SPN triples (combat-log bridge)
    db.populate_spawn_runtime_ids()?;

    // Sixth pass: derive mission identities from qst.* + mpn-prefix groupings
    db.populate_missions()?;

    // Seventh pass: structure conquest objectives by category and cadence
    db.populate_conquest_objectives()?;

    // Eighth pass: aggregate NPCs and rewards across each mission's phase tree
    db.populate_mission_data()?;

    // Ninth pass: build quest chain links from 0xCF big-endian GUID refs
    db.populate_quest_chain()?;

    // Tenth pass: build planet_transition chain links from leaving_ quest strings
    db.populate_planet_transitions()?;

    // Tenth-and-a-half pass: derive arc-order chain edges from FQN structure
    // (act_N -> act_(N+1) class story, hub_N -> hub_(N+1) world_arc).
    // SWTOR doesn't encode story-arc progression as inter-quest GUID refs --
    // it lives in FQN segment ordering. Edges land with link_type='fqn_arc_order'.
    let fqn_chain_count = db.populate_quest_chain_fqn_order()?;
    println!("  Quest chain FQN-arc edges: {}", fqn_chain_count);

    // Quest clusters for bulk curation. Each quest FQN gets one row per
    // matching cluster_kind (class_act, world_arc_hub, planet_world, etc).
    let cluster_count = db.populate_quest_clusters()?;
    println!("  Quest cluster assignments: {}", cluster_count);

    // Schematic recipe extraction (#60). Pairs each itm.schem.* with its
    // schem.* companion object and decodes the recipe (output + materials
    // with quantities) from the schem.* payload's CF GUID refs.
    let schem_count = db.populate_schematic_recipes()?;
    println!("  Schematic recipes: {}", schem_count);

    // Ability stat extraction (#69). Scans abl.* payloads for [u16 propId]
    // [f32 value] pairs in the 0x0400-0x04FF range. Verified prop IDs land
    // in dedicated columns (cooldown, cast_time, force_cost, melee_range,
    // aoe_radius, gap_closer/knockback flags); all hits land in raw_props
    // JSON for follow-up analysis of unknowns.
    let abl_stats_count = db.populate_ability_stats()?;
    println!("  Ability stats: {}", abl_stats_count);

    // Talent details (#70). FQN-derived resource_pool + tier + payload tail
    // string (script_hook). Mirrors ability_stats classification for tal.*.
    let tal_details_count = db.populate_talent_details()?;
    println!("  Talent details: {}", tal_details_count);

    // GSF talent stats (#80). Decodes c9 01 XX 01 04 <f32 LE> records
    // anchored on the cb 19 d7 4b ?? 03 signature. ~71% of tal.spvp.*
    // talents carry at least one record; the rest are flag-only effects.
    let gsf_stats_count = db.populate_gsf_talent_stats()?;
    println!("  GSF talent stats: {}", gsf_stats_count);

    // GSF base ability stats (#78). Walks abl.spvp.* payloads for scattered
    // [u16 LE prop_id][f32 LE value] records where prop_id high byte is 0x04.
    // Wide format: one row per record, consumers pivot by prop_id. ~85% of
    // abl.spvp.* abilities carry at least one record; uncovered abilities
    // are passive auras with effects on a parent activator or in a hook.
    let gsf_ability_stats_count = db.populate_gsf_ability_stats()?;
    println!("  GSF ability stats: {}", gsf_ability_stats_count);

    // EPP appearance specs + FX specs (#183). UTF-16-LE XML files. epp
    // carries appearance action lists + fxSpec refs; fxspec carries node
    // class lists. The two tables JOIN via appearance_specs.fx_spec_refs
    // → fx_specs.fqn (path-relative keys).
    let appearance_count = db.populate_appearance_specs(&args.input, &hash_dict)?;
    println!("  Appearance specs: {}", appearance_count);
    let fxspec_count = db.populate_fx_specs(&args.input, &hash_dict)?;
    println!("  FX specs: {}", fxspec_count);

    // SCPT scripts (#182). Decrypt every .scpt file in
    // /resources/systemgenerated/compilednative/ and persist the body as a
    // base64-encoded blob. Per-script semantic interpretation downstream.
    let scripts_count = db.populate_scripts(&args.input, &hash_dict)?;
    println!("  Scripts: {}", scripts_count);

    // NODE-format prototype entities (#175 cnv + #181 non-cnv). Walks every
    // PROT-magic .node file in /resources/systemgenerated/prototypes/ and
    // emits one row per file into the `objects` table. The `kind` column
    // is FQN-derived (Conversation for cnv.*, Creature for creature.*,
    // etc.). Per-prototype-class typed decoders are follow-ons.
    let node_objects = db.populate_node_objects(&args.input, &hash_dict)?;
    println!("  NODE objects: {}", node_objects);

    // Conversation refs from NODE files (cnv.* prototypes). One pass through
    // the .tor archives extracts CF GUID refs to quest, npc, achievement,
    // codex, item, follow-up conversation, and encounter targets. The
    // connective tissue for "which NPC's conversation gives/affects what".
    let cnv_refs = db.populate_conversation_refs(&args.input, &hash_dict)?;
    println!(
        "  Conversation refs: quest={} npc={} ach={} cdx={} item={} followup={} enc={} align_events={}",
        cnv_refs.quest,
        cnv_refs.npc,
        cnv_refs.achievement,
        cnv_refs.codex,
        cnv_refs.item,
        cnv_refs.followup,
        cnv_refs.encounter,
        cnv_refs.alignment_event,
    );

    // Quest_chain via NPC giver overlap. Must run AFTER both
    // populate_quest_clusters (cluster filter) and populate_conversation_refs
    // (the conv_quest_refs / conv_npcs join surface).
    let npc_chain_count = db.populate_quest_chain_npc_giver()?;
    println!("  Quest chain NPC-giver edges: {}", npc_chain_count);

    // Eleventh pass: class taxonomy (#94). Origins are hardcoded (no GOM
    // object); combat styles come from class.pc.advanced.* with display
    // names resolved through cdx.advanced_classes.*. Must run before
    // populate_disciplines so disciplines/css/cut FKs to combat_styles
    // (fqn_segment) resolve.
    let origin_count = db.populate_origins()?;
    let combat_style_count = db.populate_combat_styles()?;

    // Twelfth pass: derive disciplines, discipline_abilities, and the
    // combat_style_shared_abilities table (per-origin shared/utility/mod
    // pools, fanned to both combat styles).
    let (disc_count, disc_abl_count, css_abl_count) = db.populate_disciplines()?;

    // Twelfth-and-a-half pass: enrich disciplines with authoritative data
    // decoded from dis.* PBUK payloads (issue #170). Adds codename, icon +
    // mod-tree apc refs, signature ability, and populates discipline_mods
    // with the 8-tier × 3-choice mod tree per
    // docs/probes/dis-payload-format.md.
    let (dis_disc_count, dis_mod_count) = db.populate_disciplines_from_dis()?;

    // Twelfth-and-three-quarters pass: GSF requisition costs from the two
    // sc...Cost singletons (issue #172, closes #115). First per-singleton
    // decoder on top of the #171 singleton pipeline.
    let (gsf_component_costs, gsf_tier_costs) = db.populate_gsf_requisition_costs()?;

    // Twelfth-and-seven-eighths pass: ability/talent → effect block linkage
    // (issue #173). One row per indexed CF E0 sub-record in each abl.*/tal.*
    // payload. Unresolved block GUIDs (versioned-only ability category, #179)
    // are preserved with NULL block_game_id so the gap is visible in spice.
    let (effect_block_rows, effect_block_unresolved) = db.populate_ability_effect_blocks()?;

    // Twelfth-and-thirty-one-thirty-secondths pass: NPC typed details
    // (issue #176). Extracts class_role + ai_template from payload strings;
    // faction from FQN structure. difficulty + level remain NULL pending
    // per-property byte-layout decode work (deferred).
    let npc_details_count = db.populate_npc_details_typed()?;
    println!("  NPC details typed: {}", npc_details_count);

    // Twelfth-and-fifteen-sixteenths pass: tag dictionary + ability_tags +
    // talent_tags (issue #174). Decodes ~6750 tag.abl.* entries from the
    // tagTablePrototype singleton, then cross-references every abl/tal
    // payload (both canonical AND non-canonical variants) for hash matches.
    let (tag_count, abl_tag_edges, tal_tag_edges) = db.populate_tags_and_edges()?;

    // Thirteenth pass: derive discipline→talent + class_utility_talents.
    // Per-origin utility talents fan to both combat styles; combat-discipline
    // talents stay scoped to their own discipline (no fan-out).
    let (disc_tal_count, cut_count) = db.populate_discipline_talents()?;

    // Fourteenth pass: decode talent→ability GUID refs from tal.* payloads
    let talent_abl_count = db.populate_talent_abilities()?;

    // Print summary
    let stats = db.stats()?;
    println!("\nExtraction complete!");
    println!("  Archives: {}", tor_files.len());
    println!("  File hashes scanned: {}", seen_hashes.len());
    println!();
    println!("  Objects: {}", total_objects);
    println!(
        "    Quests: {} ({} classified, {} chain links, {} npc links, {} reward links, {} runtime ids)",
        stats.quests, quest_count, stats.chain_links, stats.npc_links, stats.reward_links, stats.runtime_ids
    );
    println!(
        "    Missions: {} ({} npcs, {} rewards)",
        stats.missions, stats.mission_npcs, stats.mission_rewards
    );
    println!("    Abilities: {}", stats.abilities);
    println!(
        "    Disciplines: {} ({} ability slots, {} talent slots, {} talent->ability links)",
        stats.disciplines, stats.discipline_abilities, disc_tal_count, stats.talent_abilities
    );
    println!(
        "    Disciplines (dis.*): {} enriched, {} mods",
        dis_disc_count, dis_mod_count
    );
    println!(
        "    GSF requisition costs: {} components + {} tier upgrades",
        gsf_component_costs, gsf_tier_costs
    );
    println!(
        "    Ability effect blocks: {} rows ({} unresolved GUIDs)",
        effect_block_rows, effect_block_unresolved
    );
    println!(
        "    Tags: {} dictionary entries, {} ability edges, {} talent edges",
        tag_count, abl_tag_edges, tal_tag_edges
    );
    println!(
        "    Combat-style shared: {} abilities, {} utility talents",
        stats.combat_style_shared_abilities, stats.class_utility_talents
    );
    println!(
        "    Class taxonomy: {} origins, {} combat styles",
        stats.origins, stats.combat_styles
    );
    let _ = (
        disc_count,
        disc_abl_count,
        css_abl_count,
        cut_count,
        talent_abl_count,
        origin_count,
        combat_style_count,
    );
    println!("    Items: {}", stats.items);
    println!("    NPCs: {}", stats.npcs);
    println!("    Conquest objectives: {}", stats.conquest_objectives);
    println!();
    println!("  Strings: {}", stats.strings);
    if args.icons {
        println!();
        println!("  Icons: {} (deduplicated)", total_icons);
        println!("    Output: {}", args.icons_output.display());
    }

    // Finalize unknowns tracker
    if let Some(ref unknowns_path) = args.unknowns {
        unknowns_writer.finalize()?;
        println!();
        println!("  Unknowns: {}", unknowns_path.display());
    }

    Ok(())
}

/// Resolve the hashes file path: use --hashes if provided, otherwise look for
/// hashes_filename.txt next to the output file, and download from Jedipedia if missing.
fn resolve_hashes_path(args: &Args) -> Result<Option<PathBuf>> {
    // Explicit path provided
    if let Some(ref path) = args.hashes {
        if path.exists() {
            return Ok(Some(path.clone()));
        }
        anyhow::bail!("Hash file not found: {}", path.display());
    }

    // Check default location: same directory as output
    let default_path = args
        .output
        .parent()
        .unwrap_or(Path::new("."))
        .join("hashes_filename.txt");

    if default_path.exists() {
        println!("Using hash dictionary: {}", default_path.display());
        return Ok(Some(default_path));
    }

    // Download from Jedipedia
    println!("Downloading hash dictionary from Jedipedia...");
    let url = "https://swtor.jedipedia.net/ajax/getFileNames.php?env=live&format=easymyp";

    let response = ureq::get(url)
        .call()
        .map_err(|e| anyhow::anyhow!("Failed to download hashes: {}", e))?;

    let mut body = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

    // Ensure parent directory exists
    if let Some(parent) = default_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&default_path, &body)?;
    println!(
        "Saved hash dictionary ({:.1} MB) to {}",
        body.len() as f64 / 1_048_576.0,
        default_path.display()
    );

    Ok(Some(default_path))
}

/// Strip `/major/minor` version suffix from a GOM FQN, or return as-is if unversioned.
/// Callers deduplicate via `accept_variant`, which keeps the highest-quality
/// variant per base FQN (preferring objects that resolved a string_id, then
/// icon_name, then larger payload).
fn normalize_fqn(fqn: &str) -> Option<String> {
    if !fqn.contains('/') {
        return Some(fqn.to_string());
    }
    let slash1 = fqn.rfind('/')?;
    let base_end = fqn[..slash1].rfind('/')?;
    Some(fqn[..base_end].to_string())
}

/// Check if a GOM object should be extracted based on FQN prefix
fn should_extract_object(fqn: &str, unfiltered: bool) -> bool {
    // Safety guard: versioned FQNs should be normalized before reaching here.
    // The extraction loops call versioned_fqn_base() first and skip non-zero minors.
    if fqn.contains('/') {
        return false;
    }

    let prefix = match fqn.find('.') {
        Some(pos) => &fqn[..pos],
        None => fqn,
    };

    // Must be a known prefix type (always applied).
    // enc/spn/plc are required for quest_npcs population: quest payloads
    // reference NPCs through encounter (enc.*) and spawn (spn.*) intermediaries
    // and through placeable (plc.*) targets. Without these, populate_quest_npcs
    // sees an empty resolution map and writes zero rows.
    if !matches!(
        prefix,
        "abl"
            | "tal"
            | "itm"
            | "npc"
            | "schem"
            | "qst"
            | "cdx"
            | "ach"
            | "mpn"
            | "pkg"
            | "loot"
            | "rew"
            | "cnv"
            | "apc"
            | "class"
            | "enc"
            | "spn"
            | "plc"
            | "epp"
            // Per kessel issue #169: surface previously-dropped PBUK
            // categories as queryable GameObjects. Each is documented in
            // docs/probes/pbuk-prefix-probes.md by category.
            | "dis"  // disciplines (authoritative; consumed by issue #170)
            | "stg"  // cutscene/mission staging
            | "hyd"  // gameplay event handlers / scene triggers
            | "cnd"  // named boolean conditions / predicate library
            | "npp"  // NPC build/spec packages / loadout templates
            | "dyn"  // dynamic boss-fight placeables (multi-state)
            | "apn"  // animation packages per NPC
            | "cos"  // cosmetic NPC archetype tags
            | "pcs"  // player character species presets
            | "nco"  // NPC companions, era-tagged
            | "mrp"  // mount/reward packages, event-tied
            | "ipp" // item paint/pattern prototypes
    ) {
        return false;
    }

    let parts: Vec<&str> = fqn.split('.').collect();

    // Scope `epp.*` extraction to player abilities, companion abilities, and
    // boss/encounter content the ground EPIC (#177) and ops-guide
    // choreography goal need. Excluded: epp.npc.* (NPC ability internals),
    // epp.world_design.*, epp.placeables.*, epp.test.*, epp.creature.*, etc.
    //
    // Class prefix in source is the SHORT form (e.g. `epp.agent.*`, not
    // `epp.imperial_agent.*`). Mirrors the convention already used by
    // populate_disciplines, which derives class_code from `abl.<class>.skill.*`
    // FQNs and ends up with class_code = "agent", "sith_inquisitor", etc.
    if prefix == "epp" && parts.len() >= 3 {
        let second = parts[1];
        let third = parts[2];
        let is_player_class = matches!(
            second,
            "sith_warrior"
                | "sith_inquisitor"
                | "bounty_hunter"
                | "agent"
                | "jedi_knight"
                | "jedi_consular"
                | "smuggler"
                | "trooper"
        );
        let is_shared_flurry = second == "flurry" && matches!(third, "melee" | "ranged");
        // Companion abilities, boss encounters, expansion-specific
        // encounter abilities (Iokath/Dxun/Oricon/etc.) and daily-area
        // mechanics. Story-arc content lives under epp.exp.*.
        let is_encounter_or_companion = matches!(
            second,
            "companion" | "flashpoint" | "operation" | "qtr" | "daily_area" | "exp" | "spvp"
        );
        if !(is_player_class || is_shared_flurry || is_encounter_or_companion) {
            return false;
        }
    }

    // Skip test, debug, deprecated content (always applied - this is garbage)
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

    // When --unfiltered, skip content-based filtering and let ETL handle it
    if unfiltered {
        return true;
    }

    // Content-based filters below (only applied when NOT unfiltered)
    // These can be replicated in ETL scripts for finer control

    // Skip internal abilities
    if prefix == "abl" && parts.len() >= 2 {
        let second = parts[1];

        // Allowlist `abl.itm.tactical.*`, `abl.itm.setbonus.*`, and
        // `abl.itm.legendary.*` BEFORE the generic `abl.itm.*` blocklist.
        // These three carry the mechanical-effect strings Jedipedia exposes
        // on tactical / set-bonus / legendary-implant pages. The item's
        // payload references this abl.* by GUID; the abl.* row's string_id
        // holds the effect description text in str.abl.1.<id>. Without this
        // allowlist the abl.itm generic blocklist below drops them and the
        // wiring is invisible (see #111).
        //
        // Source-canon segment names verified against Jedipedia ability URLs:
        //   abl.itm.tactical.<source>.<class_group>.<style_group>.<modifier_id>
        //   abl.itm.setbonus.<source>.<class_group>.<bonus_id>_NN  (NN = tier rank)
        //   abl.itm.legendary.<source>.<class_group>.<modifier_id>
        if second == "itm" && parts.len() >= 3 {
            let third = parts[2];
            if matches!(third, "tactical" | "setbonus" | "legendary") {
                return true;
            }
        }

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
                // Removed `"flurry"` from this blocklist (was here pre-2026-05-04).
                // Jedipedia shows player base-class abilities like Saber Strike
                // (string_id 220604) live at abl.flurry.npc.<class>_stance_flurry --
                // dropping all of abl.flurry.* hides those fundamentals (tracked
                // historically by #57). The companion epp side already uses
                // is_shared_flurry above to allow epp.flurry.melee/ranged.
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

/// True for FQNs that are PBUK singleton prototypes (zero-dot PascalCase /
/// camelCase identifiers like `tagTablePrototype`, `colCollectionItemsPrototype`).
/// These bypass `should_extract_object`'s prefix whitelist and route to the
/// `singletons` table instead of `objects`.
fn is_singleton_fqn(fqn: &str) -> bool {
    !fqn.is_empty() && !fqn.contains('.') && !fqn.contains('/')
}

fn process_pbuk(
    data: &[u8],
    db: &db::Database,
    unfiltered: bool,
    overrides: Option<&icon_overrides::IconOverrides>,
    versioned_seen: &mut HashMap<String, u64>,
) -> Result<usize> {
    let objects = pbuk::parse(data)?;
    let mut count = 0;

    for mut obj in objects {
        let Some(fqn) = normalize_fqn(&obj.fqn) else {
            continue;
        };
        obj.fqn = fqn.clone();

        // Singletons (zero-dot PBUK objects) route to their own table.
        // Per kessel issue #171: they're master tables / config blobs whose
        // shape doesn't match the per-instance GameObject model.
        if is_singleton_fqn(&fqn) {
            db.insert_singleton(&obj)?;
            count += 1;
            continue;
        }

        let game_obj = schema::GameObject::from_gom_with_overrides(&obj, overrides);
        if !accept_variant(versioned_seen, &fqn, &game_obj) {
            continue;
        }
        if should_extract_object(&game_obj.fqn, unfiltered) && !game_obj.fqn.is_empty() {
            db.insert_object(&game_obj)?;
            count += 1;
        }
    }

    Ok(count)
}

/// Score a candidate object: prefer those that extracted a string_id, then
/// those with an icon_name, then larger payloads. Returns a single u64 so
/// callers can compare with `>`.
fn score_variant(obj: &schema::GameObject) -> u64 {
    let payload_size = obj
        .json
        .get("payload_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let has_string = obj.string_id.is_some() as u64;
    let has_icon = obj.icon_name.is_some() as u64;
    (has_string << 40) + (has_icon << 30) + payload_size
}

/// Return `true` if `obj` is a strictly better variant for this FQN than
/// anything seen so far, updating the per-FQN best score. Inferior or
/// duplicate-quality variants return `false` and should be skipped. A
/// post-extraction SQL dedup collapses any remaining multi-GUID rows.
fn accept_variant(seen: &mut HashMap<String, u64>, fqn: &str, obj: &schema::GameObject) -> bool {
    let score = score_variant(obj);
    match seen.get(fqn).copied() {
        Some(prev) if prev >= score => false,
        _ => {
            seen.insert(fqn.to_string(), score);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_extract_object_accepts_new_whitelisted_prefixes() {
        // Per kessel issue #169: 11 previously-dropped PBUK prefixes are now
        // accepted. One representative FQN per prefix per
        // docs/probes/pbuk-prefix-probes.md.
        let samples = [
            "dis.powertech.firebug",
            "stg.location.hoth.class.spy.supply_cache_a",
            "hyd.location.nar_shaddaa.mob.hub2.green.rep1.room02.poi00",
            "cnd.itm.has_item.lots.armor.bh_tro_dps",
            "npp.location.makeb.mercenaries.infantry_bms_elite",
            "dyn.operation.iokath.boss.scyva.combat.railgun_cove_blue",
            "apn.npc.qtr.1x1.raid.karaggas_palace.enemy.trash",
            "cos.location.tatooine.mob.refurbished_militia_droid",
            "pcs.jedi_knight.female.rattataki_legacy",
            "nco.companions_original.warrior.broonmark",
            "mrp.galactic_seasons.season_5.mouse_droid",
            "ipp.custom.sow.progresson.ge_a13_purple_holo.legs",
        ];
        for fqn in samples {
            assert!(
                should_extract_object(fqn, false),
                "{fqn} must be accepted by the post-#169 whitelist"
            );
        }
    }

    #[test]
    fn should_extract_object_still_rejects_unknown_prefix() {
        assert!(!should_extract_object("zzzunknownprefix.foo", false));
        assert!(!should_extract_object("randomgarbage", false));
    }

    #[test]
    fn should_extract_object_still_rejects_versioned_fqns() {
        // The /N/M suffix gate is unchanged; versioned FQNs still get
        // normalized at a different layer.
        assert!(!should_extract_object("abl.foo.bar/7/0", false));
    }

    #[test]
    fn is_singleton_fqn_classifies_correctly() {
        // Per kessel issue #171: zero-dot, non-slash, non-empty FQNs are
        // singleton prototypes (master tables / config blobs).
        assert!(is_singleton_fqn("tagTablePrototype"));
        assert!(is_singleton_fqn("colCollectionItemsPrototype"));
        assert!(is_singleton_fqn("Suburb"));
        assert!(is_singleton_fqn("cnqConquestInfoPrototype"));
        // Per-instance objects (dotted) are NOT singletons.
        assert!(!is_singleton_fqn("abl.sith_warrior.ravage"));
        assert!(!is_singleton_fqn("dis.powertech.firebug"));
        // Versioned variants are filtered before this check fires, but
        // defensively the slash-containing case is rejected.
        assert!(!is_singleton_fqn("abl.foo.bar/7/0"));
        assert!(!is_singleton_fqn(""));
    }

    #[test]
    fn should_extract_object_preserves_existing_accepts() {
        // Regression guard for the existing 19 whitelisted prefixes.
        for fqn in [
            "abl.sith_warrior.ravage",
            "tal.spvp.laser.rapid_fire_laser.tier_4a",
            "itm.eq.legacy.weapon.lightsaber.darth_marrs",
            "npc.companion.t7-o1",
            "schem.armor.synthweaving.tier1",
            "qst.location.alderaan.class.sith_warrior.battle_organa",
            "cdx.persons.ilum.supreme_commander_rans",
            "ach.operations.iokath.hardmode16.kill_izax",
            "mpn.location.open_worlds.class.jedi_knight",
            "pkg.profession_trainer.synthweaving_base",
            "cnv.location.nar_shaddaa.class.sith_warrior.general_kligton",
            "apc.companion.class.mtx.creature.nathema_voreclaw.healer",
            "class.pc.advanced.sorcerer",
            "enc.flashpoint.manaan.boss.enc_ortuno",
            "spn.qtr.1x4.raid.asation.enemy.trash.ruins.ruins_assassin",
            "plc.location.belsavis.class.trooper.multi.ship_holoterminal",
        ] {
            assert!(
                should_extract_object(fqn, false),
                "{fqn} must continue to be accepted (regression guard)"
            );
        }
    }
}
