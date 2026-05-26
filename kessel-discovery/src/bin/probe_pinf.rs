//! Re-investigate PINF (prototypes.info) format. Issue #180.
//!
//! Reads the PINF file from any .tor archive in the input dir, runs the
//! existing parser, then reports:
//! - record count
//! - flag (byte 8) histogram
//! - unknown byte (byte 9) histogram
//! - cross-byte (8,9) joint distribution top entries
//! - cross-reference against .node hash dictionary (orphans + missing)
use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use kessel::prototypes_info;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut hd = HashDictionary::new();
    hd.load(&PathBuf::from(
        "/Users/seansilvius/.cache/kessel/hashes_filename.txt",
    ))?;
    let pinf_hash = hd
        .paths_matching("/resources/systemgenerated/prototypes.info")
        .into_iter()
        .map(|(h, _)| h)
        .next();
    let pinf_hash = pinf_hash.expect("PINF path not in dictionary");
    eprintln!("PINF hash: {pinf_hash:016X}");

    let tor_files: Vec<_> = std::fs::read_dir(&PathBuf::from("/Users/seansilvius/swtor/Assets"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut pinf_bytes: Option<Vec<u8>> = None;
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
            if entry.filename_hash == pinf_hash {
                if let Ok(data) = archive.read_entry(entry) {
                    pinf_bytes = Some(data);
                    break;
                }
            }
        }
        if pinf_bytes.is_some() {
            break;
        }
    }
    let bytes = pinf_bytes.expect("PINF not found in any archive");
    eprintln!("PINF size: {} bytes", bytes.len());

    let records = prototypes_info::parse(&bytes)?;
    eprintln!("records: {}", records.len());

    // Flag histogram (byte 8)
    let mut flag_hist: BTreeMap<u8, u64> = BTreeMap::new();
    for r in &records {
        *flag_hist.entry(r.flag).or_insert(0) += 1;
    }
    let nonzero = flag_hist.iter().filter(|(_, n)| **n > 0).count();
    eprintln!("\n--- flag byte distribution ---");
    eprintln!("distinct flag values: {} (out of 256 possible)", nonzero);
    let mut top: Vec<_> = flag_hist.iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (b, n) in top.iter().take(20) {
        eprintln!("  0x{:02X}  {}", b, n);
    }

    // Pull byte9 too. Re-parse manually.
    let mut byte9_hist: BTreeMap<u8, u64> = BTreeMap::new();
    let mut joint: BTreeMap<(u8, u8), u64> = BTreeMap::new();
    let header_len = 11;
    let record_len = 10;
    let mut i = header_len;
    while i + record_len <= bytes.len() {
        let chunk = &bytes[i..i + record_len];
        let f = chunk[8];
        let u = chunk[9];
        *byte9_hist.entry(u).or_insert(0) += 1;
        *joint.entry((f, u)).or_insert(0) += 1;
        i += record_len;
    }
    let b9_nonzero = byte9_hist.iter().filter(|(_, n)| **n > 0).count();
    eprintln!("\n--- byte 9 distribution ---");
    eprintln!("distinct values: {}", b9_nonzero);
    let mut b9_top: Vec<_> = byte9_hist.iter().collect();
    b9_top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (b, n) in b9_top.iter().take(10) {
        eprintln!("  0x{:02X}  {}", b, n);
    }

    eprintln!("\n--- top 10 (flag, byte9) joint pairs ---");
    let mut joint_top: Vec<_> = joint.iter().collect();
    joint_top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for ((f, u), n) in joint_top.iter().take(10) {
        eprintln!("  flag=0x{:02X} byte9=0x{:02X}  {}", f, u, n);
    }

    // .node cross-reference: each .node file's PROT header carries a
    // content GUID at bytes 8..16 (LE). Walk the archive entries to build a
    // content_guid -> numeric_id map, then check PINF coverage.
    let extant_node_guids: HashSet<String> = {
        let proto_paths_hashes: HashSet<u64> = hd
            .paths_matching("/resources/systemgenerated/prototypes/")
            .into_iter()
            .filter_map(|(h, p)| if p.ends_with(".node") { Some(h) } else { None })
            .collect();
        let mut guids = HashSet::new();
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
                if !proto_paths_hashes.contains(&entry.filename_hash) {
                    continue;
                }
                if let Ok(data) = archive.read_entry(entry) {
                    if data.len() >= 16 && &data[..4] == b"PROT" {
                        let g: [u8; 8] = data[8..16].try_into().unwrap();
                        guids.insert(format!("{:016X}", u64::from_le_bytes(g)));
                    }
                }
            }
        }
        guids
    };
    eprintln!("\n--- node cross-ref ---");
    eprintln!(
        ".node content GUIDs in archives: {}",
        extant_node_guids.len()
    );
    let pinf_guids: HashSet<String> = records.iter().map(|r| r.content_guid.clone()).collect();
    let with_node = pinf_guids.intersection(&extant_node_guids).count();
    eprintln!(
        "PINF records whose GUID matches a .node file: {} ({:.1}%)",
        with_node,
        100.0 * with_node as f64 / records.len() as f64
    );
    let node_without_pinf = extant_node_guids.difference(&pinf_guids).count();
    eprintln!(".node files without a PINF entry: {}", node_without_pinf);

    let mut flag_extant: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
    for r in &records {
        let entry = flag_extant.entry(r.flag).or_insert((0, 0));
        entry.0 += 1;
        if extant_node_guids.contains(&r.content_guid) {
            entry.1 += 1;
        }
    }
    eprintln!("\n--- per-flag .node-extant rate ---");
    let mut fe: Vec<_> = flag_extant.iter().collect();
    fe.sort_by_key(|(_, (total, _))| std::cmp::Reverse(*total));
    for (flag, (total, extant)) in fe.iter() {
        let pct = if *total > 0 {
            100.0 * *extant as f64 / *total as f64
        } else {
            0.0
        };
        eprintln!(
            "  flag=0x{:02X}  {} records, {} have .node ({:.1}%)",
            flag, total, extant, pct
        );
    }
    Ok(())
}
