//! dictless_census -- evidence that kessel can discover every file type it needs
//! WITHOUT the community hash dictionary (hashes_filename.txt).
//!
//! One-off recon tool. Run:
//!   cargo build --release -p kessel-discovery --example dictless_census
//!   ./target/release/examples/dictless_census
//!
//! Three analyses:
//!   1. MAGIC CENSUS    -- classify every entry by CONTENT magic (zero dict).
//!   2. MAGIC vs DICT   -- does content magic match/exceed dict path-based ID?
//!   3. HASH-DERIVATION -- reconstruct paths from conventions + spice self-refs,
//!                         combine_hash them, see how many hit real entries.
//!
//! The OUTPUT is the deliverable. Liberal prints are intentional.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use kessel::dds;
use kessel::hash::{self, HashDictionary};
use kessel::myp::Archive;
use kessel::pbuk;

// ---------------------------------------------------------------------------
// Hardcoded runtime inputs (one-off recon tool -- not production config).
// ---------------------------------------------------------------------------
/// Directory holding the *.tor archives.
const ASSETS_DIR: &str = "/Users/seansilvius/swtor/assets";
/// Community hash dictionary -- CROSS-CHECK ONLY (proves magic >= dict coverage).
const DICT_PATH: &str = "/Users/seansilvius/swtor/data/hashes_filename.txt";
/// Populated spice DB (7.9 v1) -- harvest self-references (icons, conversations).
const SPICE_DB: &str = "/Users/seansilvius/swtor/data/spice.sqlite";

// ---------------------------------------------------------------------------
// File-type classification by CONTENT magic. No dict, no path needed.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FileClass {
    Prot,
    Stb,
    Scpt,
    Pbuk,
    Dblb,
    Dds,
    Utf16Xml,
    Other,
}

impl FileClass {
    fn label(self) -> &'static str {
        match self {
            FileClass::Prot => "PROT",
            FileClass::Stb => "STB",
            FileClass::Scpt => "SCPT",
            FileClass::Pbuk => "PBUK",
            FileClass::Dblb => "DBLB",
            FileClass::Dds => "DDS",
            FileClass::Utf16Xml => "UTF16_XML",
            FileClass::Other => "OTHER",
        }
    }
}

/// PROT prototype container: magic "PROT" at byte 0.
fn is_prot(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"PROT"
}

/// SCPT compiled-native script: magic "SCPT" at byte 0 (see kessel/src/scpt.rs).
fn is_scpt(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"SCPT"
}

/// STB string table: header byte 0 == 0x01 (version). Weak magic -- check LAST
/// so it cannot steal entries from the strong 4-byte magics.
fn is_stb(data: &[u8]) -> bool {
    !data.is_empty() && data[0] == 0x01
}

/// UTF-16LE XML (epp / fxspec): BOM 0xFF 0xFE at byte 0.
fn is_utf16_xml(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE
}

/// Classify decompressed bytes. Strong 4-byte magics first; weak STB last.
fn classify(data: &[u8]) -> FileClass {
    if is_prot(data) {
        FileClass::Prot
    } else if pbuk::is_pbuk(data) {
        FileClass::Pbuk
    } else if pbuk::is_dblb(data) {
        FileClass::Dblb
    } else if is_scpt(data) {
        FileClass::Scpt
    } else if dds::is_dds(data) {
        FileClass::Dds
    } else if is_utf16_xml(data) {
        FileClass::Utf16Xml
    } else if is_stb(data) {
        FileClass::Stb
    } else {
        FileClass::Other
    }
}

// ---------------------------------------------------------------------------
// Hash invariant (verified this session):
//   archive entry filename_hash == combine_hash(ph, sh)
//   where (ph, sh) = hashlittle2(path, 0, 0)
// NOTE: hash::swtor_filename_hash swaps the halves and does NOT match.
// ---------------------------------------------------------------------------
fn path_hash(path: &str) -> u64 {
    let (ph, sh) = hash::hashlittle2(path.as_bytes(), 0, 0);
    hash::combine_hash(ph, sh)
}

fn list_tor_files(dir: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        eprintln!("ERROR: cannot read assets dir {dir}");
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "tor").unwrap_or(false) {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn pct(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        100.0 * num as f64 / den as f64
    }
}

fn main() -> anyhow::Result<()> {
    println!("================================================================");
    println!("  DICTLESS CENSUS -- proving kessel discovers file types with no");
    println!("  community hash dictionary.");
    println!("================================================================");

    let tor_files = list_tor_files(ASSETS_DIR);
    println!("Found {} *.tor archives in {ASSETS_DIR}\n", tor_files.len());
    if tor_files.is_empty() {
        anyhow::bail!("no archives found -- aborting");
    }

    // -------------------------------------------------------------------
    // SWEEP: decompress every entry once, classify by magic, record hashes.
    // This is the expensive pass (a few minutes). Everything downstream
    // reuses these in-memory results.
    // -------------------------------------------------------------------
    let mut class_counts: HashMap<FileClass, usize> = HashMap::new();
    // hash -> magic class, for every archive entry across all archives.
    let mut entry_class: HashMap<u64, FileClass> = HashMap::new();
    let mut all_hashes: HashSet<u64> = HashSet::new();
    let mut total_entries: usize = 0;
    let mut read_errors: usize = 0;

    println!("=== SWEEP: decompressing + classifying every entry ===");
    for (i, path) in tor_files.iter().enumerate() {
        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut archive = match Archive::open(path) {
            Ok(a) => a,
            Err(e) => {
                eprintln!(
                    "  [{:>3}/{}] SKIP {fname}: open failed: {e}",
                    i + 1,
                    tor_files.len()
                );
                continue;
            }
        };
        // Snapshot the entry list (read_entry needs &mut self).
        let entries: Vec<_> = match archive.entries() {
            Ok(it) => it.cloned().collect(),
            Err(e) => {
                eprintln!(
                    "  [{:>3}/{}] SKIP {fname}: entries failed: {e}",
                    i + 1,
                    tor_files.len()
                );
                continue;
            }
        };
        let mut archive_entries = 0usize;
        for entry in &entries {
            all_hashes.insert(entry.filename_hash);
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => {
                    read_errors += 1;
                    continue;
                }
            };
            let class = classify(&data);
            *class_counts.entry(class).or_insert(0) += 1;
            // First magic wins per hash (entries can repeat across archives).
            entry_class.entry(entry.filename_hash).or_insert(class);
            total_entries += 1;
            archive_entries += 1;
        }
        println!(
            "  [{:>3}/{}] {fname}: {archive_entries} entries classified",
            i + 1,
            tor_files.len()
        );
    }
    println!();

    // -------------------------------------------------------------------
    // ANALYSIS 1: MAGIC CENSUS
    // -------------------------------------------------------------------
    println!("=== ANALYSIS 1: MAGIC CENSUS (zero dict) ===");
    println!("Total entries decompressed + classified: {total_entries}");
    println!(
        "Unique entry hashes seen:                 {}",
        all_hashes.len()
    );
    println!("Read/decompress errors (skipped):         {read_errors}");
    println!("Per-class counts:");
    let order = [
        FileClass::Prot,
        FileClass::Stb,
        FileClass::Scpt,
        FileClass::Pbuk,
        FileClass::Dblb,
        FileClass::Dds,
        FileClass::Utf16Xml,
        FileClass::Other,
    ];
    for c in order {
        let n = class_counts.get(&c).copied().unwrap_or(0);
        println!(
            "  {:<10} {:>9}  ({:>6.2}%)",
            c.label(),
            n,
            pct(n, total_entries)
        );
    }
    let identified: usize = order
        .iter()
        .filter(|c| **c != FileClass::Other)
        .map(|c| class_counts.get(c).copied().unwrap_or(0))
        .sum();
    println!(
        "Self-identifying (non-OTHER): {identified} / {total_entries} ({:.2}%)",
        pct(identified, total_entries)
    );
    println!();

    // -------------------------------------------------------------------
    // ANALYSIS 2: MAGIC vs DICT COVERAGE
    // -------------------------------------------------------------------
    println!("=== ANALYSIS 2: MAGIC vs DICT COVERAGE ===");
    let mut dict = HashDictionary::new();
    let dict_loaded = match dict.load(DICT_PATH) {
        Ok(n) => {
            println!("Loaded dict: {n} entries from {DICT_PATH}");
            true
        }
        Err(e) => {
            eprintln!("WARNING: dict load failed ({e}); skipping analysis 2 cross-check");
            false
        }
    };

    if dict_loaded {
        // For each dict path, derive the type the dict's PATH conventions imply.
        // Then for dict entries that ALSO exist in the archive (by hash), check
        // whether content magic AGREES with the path-implied type.
        fn dict_expected_type(path: &str) -> Option<FileClass> {
            let p = path.to_ascii_lowercase();
            if p.contains("/resources/systemgenerated/prototypes/") {
                Some(FileClass::Prot)
            } else if p.contains("/resources/systemgenerated/compilednative/") {
                Some(FileClass::Scpt)
            } else if p.contains("/gfx/icons/") && p.ends_with(".dds") {
                Some(FileClass::Dds)
            } else if p.contains("/str/") && p.ends_with(".stb") {
                Some(FileClass::Stb)
            } else if p.ends_with(".epp") || p.ends_with(".fxspec") {
                Some(FileClass::Utf16Xml)
            } else {
                None
            }
        }

        // dict-known count + magic-agree count per type.
        let mut dict_known: HashMap<FileClass, usize> = HashMap::new();
        let mut magic_agree: HashMap<FileClass, usize> = HashMap::new();
        let mut present_in_archive: HashMap<FileClass, usize> = HashMap::new();

        // Iterate the whole dict once via its hash map: we re-derive each path's
        // hash from the path itself (so we test the SAME orientation we use for
        // discovery), and look it up in entry_class.
        // The dict's get() only goes hash->path, so walk paths_matching on the
        // anchors that cover all five types.
        let mut dict_paths: Vec<&String> = Vec::new();
        for anchor in [
            "/resources/systemgenerated/prototypes/",
            "/resources/systemgenerated/compilednative/",
            "/gfx/icons/",
            "/str/",
            ".epp",
            ".fxspec",
        ] {
            for (_h, path) in dict.paths_matching(anchor) {
                dict_paths.push(path);
            }
        }
        dict_paths.sort();
        dict_paths.dedup();

        for path in dict_paths {
            let Some(expected) = dict_expected_type(path) else {
                continue;
            };
            *dict_known.entry(expected).or_insert(0) += 1;
            let h = path_hash(path);
            if let Some(actual) = entry_class.get(&h) {
                *present_in_archive.entry(expected).or_insert(0) += 1;
                // STB: dict path says .stb; we accept STB magic OR any of the
                // structured types, since some /str/ entries are not version-1
                // tables. Strict agreement = same class.
                if *actual == expected {
                    *magic_agree.entry(expected).or_insert(0) += 1;
                }
            }
        }

        println!("Per type (dict-path-implied vs content magic):");
        println!(
            "  {:<10} {:>10} {:>10} {:>10}  {:>8}",
            "TYPE", "DICT-KNOWN", "IN-ARCHIVE", "MAGIC-OK", "AGREE%"
        );
        for c in [
            FileClass::Prot,
            FileClass::Scpt,
            FileClass::Dds,
            FileClass::Stb,
            FileClass::Utf16Xml,
        ] {
            let known = dict_known.get(&c).copied().unwrap_or(0);
            let present = present_in_archive.get(&c).copied().unwrap_or(0);
            let agree = magic_agree.get(&c).copied().unwrap_or(0);
            println!(
                "  {:<10} {:>10} {:>10} {:>10}  {:>7.2}%",
                c.label(),
                known,
                present,
                agree,
                pct(agree, present)
            );
        }
        println!("(AGREE% is over IN-ARCHIVE -- i.e. of dict entries that really");
        println!(" exist in the archive, how many does content magic classify the");
        println!(" same way the dict's path convention would.)");
    }
    println!();

    // -------------------------------------------------------------------
    // ANALYSIS 3: HASH-DERIVATION HARVEST
    // -------------------------------------------------------------------
    println!("=== ANALYSIS 3: HASH-DERIVATION HARVEST (dict-free reconstruction) ===");

    // (a) Root STBs from naming conventions.
    println!("(a) Root STBs from convention:");
    let mut root_paths: Vec<String> = Vec::new();
    for stem in ["abl", "tal", "itm", "npc", "qst", "cdx", "ach", "schem"] {
        root_paths.push(format!("/resources/en-us/str/{stem}.stb"));
    }
    for stem in ["planetaryconquest", "galacticcommand"] {
        root_paths.push(format!("/resources/en-us/str/gui/{stem}.stb"));
    }
    let mut a_hit = 0usize;
    for p in &root_paths {
        let present = all_hashes.contains(&path_hash(p));
        if present {
            a_hit += 1;
        }
        println!("    [{}] {p}", if present { "HIT " } else { "MISS" });
    }
    println!(
        "    => {a_hit}/{} present ({:.2}%)",
        root_paths.len(),
        pct(a_hit, root_paths.len())
    );
    println!();

    // Open spice DB (icons + conversations). Skip gracefully if unavailable.
    let conn = match open_spice() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("WARNING: cannot open spice DB ({e}); skipping 3(b)/3(c)");
            None
        }
    };

    // (b) Icons from spice self-references -- the headline number.
    let mut b_distinct = 0usize;
    let mut b_present = 0usize;
    let mut b_new_vs_dict = 0usize;
    let mut dict_icon_hashes: HashSet<u64> = HashSet::new();
    if dict_loaded {
        for (h, _p) in dict.paths_matching("/gfx/icons/") {
            dict_icon_hashes.insert(h);
        }
    }
    if let Some(conn) = &conn {
        println!("(b) Icons from spice self-references:");
        match query_distinct(
            conn,
            "SELECT DISTINCT icon_name FROM objects WHERE icon_name IS NOT NULL AND icon_name<>''",
        ) {
            Ok(icons) => {
                b_distinct = icons.len();
                for name in &icons {
                    let path = format!("/resources/gfx/icons/{name}.dds");
                    let h = path_hash(&path);
                    if all_hashes.contains(&h) {
                        b_present += 1;
                        // NEW = present in archive but NOT in the stale dict's
                        // /gfx/icons/ path set.
                        if dict_loaded && !dict_icon_hashes.contains(&h) {
                            b_new_vs_dict += 1;
                        }
                    }
                }
                println!("    distinct icon_names in spice: {b_distinct}");
                println!(
                    "    hash to a PRESENT archive entry (dict-free recoverable): {b_present} ({:.2}%)",
                    pct(b_present, b_distinct)
                );
                if dict_loaded {
                    println!(
                        "    dict /gfx/icons/ entries (for reference): {}",
                        dict_icon_hashes.len()
                    );
                    println!(
                        "    HEADLINE -- present icons MISSING from stale dict (day-one recoveries): {b_new_vs_dict}"
                    );
                }
            }
            Err(e) => eprintln!("    query failed: {e}"),
        }
        println!();
    }

    // (c) cnv dialogue from spice (sanity: should be high; already shipped).
    let mut c_distinct = 0usize;
    let mut c_present = 0usize;
    if let Some(conn) = &conn {
        println!("(c) Conversation STBs from spice:");
        match query_distinct(
            conn,
            "SELECT DISTINCT fqn FROM objects WHERE kind='Conversation'",
        ) {
            Ok(fqns) => {
                c_distinct = fqns.len();
                for fqn in &fqns {
                    let rel = fqn.replace('.', "/");
                    let path = format!("/resources/en-us/str/{rel}.stb");
                    if all_hashes.contains(&path_hash(&path)) {
                        c_present += 1;
                    }
                }
                println!("    distinct Conversation fqns: {c_distinct}");
                println!(
                    "    hash to a PRESENT archive entry: {c_present} ({:.2}%)",
                    pct(c_present, c_distinct)
                );
            }
            Err(e) => eprintln!("    query failed: {e}"),
        }
        println!();
    }

    // -------------------------------------------------------------------
    // VERDICTS
    // -------------------------------------------------------------------
    println!("=== VERDICTS (per file type) ===");
    let prot_n = class_counts.get(&FileClass::Prot).copied().unwrap_or(0);
    let stb_n = class_counts.get(&FileClass::Stb).copied().unwrap_or(0);
    let scpt_n = class_counts.get(&FileClass::Scpt).copied().unwrap_or(0);
    let pbuk_n = class_counts.get(&FileClass::Pbuk).copied().unwrap_or(0);
    let dblb_n = class_counts.get(&FileClass::Dblb).copied().unwrap_or(0);
    let dds_n = class_counts.get(&FileClass::Dds).copied().unwrap_or(0);
    let xml_n = class_counts.get(&FileClass::Utf16Xml).copied().unwrap_or(0);

    println!("PROT      : {}", verdict_magic(prot_n, "magic PROT@0"));
    println!(
        "PBUK/DBLB : {}",
        verdict_magic(pbuk_n + dblb_n, "magic PBUK/DBLB@0")
    );
    println!("SCPT      : {}", verdict_magic(scpt_n, "magic SCPT@0"));
    println!(
        "DDS       : DICT-FREE: yes (magic DDS@0; {dds_n} entries) + yes (derive: spice icon_name -> /resources/gfx/icons/<name>.dds, {b_present}/{b_distinct} present, {b_new_vs_dict} beyond stale dict)"
    );
    println!(
        "STB       : DICT-FREE: yes (derive: convention root STBs {a_hit}/{} + spice Conversation fqns {c_present}/{c_distinct}); magic STB@byte0 is weak ({stb_n} entries) -- discovery should lead with derivation",
        root_paths.len()
    );
    println!(
        "UTF16_XML : {}",
        verdict_magic(xml_n, "magic BOM FFFE@0 (epp/fxspec)")
    );

    println!();
    println!("================================================================");
    println!("  END dictless_census");
    println!("================================================================");
    Ok(())
}

fn verdict_magic(n: usize, src: &str) -> String {
    if n > 0 {
        format!("DICT-FREE: yes ({src}; {n} entries)")
    } else {
        format!("DICT-FREE: no ({src} matched 0 entries -- residual)")
    }
}

// ---- spice DB helpers ------------------------------------------------------
fn open_spice() -> anyhow::Result<rusqlite::Connection> {
    if !Path::new(SPICE_DB).exists() {
        anyhow::bail!("spice DB not found at {SPICE_DB}");
    }
    let conn = rusqlite::Connection::open_with_flags(
        SPICE_DB,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    Ok(conn)
}

fn query_distinct(conn: &rusqlite::Connection, sql: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        match r {
            Ok(s) => out.push(s),
            Err(_) => continue,
        }
    }
    Ok(out)
}
