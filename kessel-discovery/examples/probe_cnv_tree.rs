//! Spike #284: map the cnv NODE dialogue-graph layout BY SHAPE.
//!
//! For a set of target cnv FQNs, locate their PROT node payload, decode the
//! GOM object stream, and dump the structure so we can characterize:
//!   (a) the dialogue-line property (line-id u32 BE + str.cnv ref),
//!   (b) the speaker/actor ref property,
//!   (c) player-OPTION vs NPC-line marker,
//!   (d) the branch-target link between option and next node.
//!
//! Usage:
//!   probe_cnv_tree -i ~/swtor/assets -H ~/swtor/data/hashes_filename.txt \
//!       cnv.alliance.nar_shaddaa.misc.public_taxi ...

use anyhow::Result;
use kessel::gom_reader::{read_object_fields, GomValue, Reader};
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::collections::HashSet;
use std::path::PathBuf;

const FIELD_MARKER: [u8; 4] = [0xCF, 0x40, 0x00, 0x00];

/// Walk the payload as a FLAT sequence of top-level `<field_id><tag><value>`
/// triples, starting at the first CF40 marker. Returns (field_id, byte_offset,
/// value) for each top-level field. This is the node payload's real shape: a
/// serialized object's field list, NOT a single wrapped object.
fn walk_top_fields(payload: &[u8]) -> Vec<(u64, usize, Result<GomValue>)> {
    let mut out = Vec::new();
    let start = match payload.windows(4).position(|w| w == FIELD_MARKER) {
        Some(s) => s,
        None => return out,
    };
    let mut reader = Reader::new(payload, start);
    loop {
        let off = reader.pos();
        let field_id = match reader.read_number() {
            Ok(v) => v,
            Err(_) => break,
        };
        let tag = match reader.read_tag() {
            Ok(t) => t,
            Err(_) => break,
        };
        let val = reader.read_value(tag);
        let failed = val.is_err();
        out.push((field_id, off, val));
        if failed {
            break;
        }
        if reader.pos() >= payload.len() {
            break;
        }
        // Realign to the next CF40 marker if the stream has interstitial bytes.
        let p = reader.pos();
        if payload.get(p..p + 4) != Some(&FIELD_MARKER) {
            match payload[p..].windows(4).position(|w| w == FIELD_MARKER) {
                Some(rel) => reader = Reader::new(payload, p + rel),
                None => break,
            }
        }
    }
    out
}

/// Summarize a GomValue compactly for one-line printing.
fn summ(v: &GomValue) -> String {
    match v {
        GomValue::Null => "null".into(),
        GomValue::U64(x) => format!("u64({x})"),
        GomValue::I64(x) => format!("i64({x})"),
        GomValue::Bool(b) => format!("bool({b})"),
        GomValue::F32(f) => format!("f32({f})"),
        GomValue::Enum(e) => format!("enum({e})"),
        GomValue::Str(s) => {
            let s = if s.len() > 60 { &s[..60] } else { s };
            format!("str({s:?})")
        }
        GomValue::List(l) => format!("list[{}]", l.len()),
        GomValue::Map(m) => format!("map[{}]", m.len()),
        GomValue::Embedded(f) => format!("obj{{{} fields}}", f.len()),
        GomValue::ClassRef(r) => format!("classref(0x{r:016X})"),
    }
}

/// Recursive structured dump with field-id low32 labels and depth guard.
fn dump(v: &GomValue, depth: usize, indent: usize) {
    let pad = "  ".repeat(indent);
    match v {
        GomValue::Embedded(fields) => {
            for (id, fv) in fields {
                let low = *id as u32;
                println!("{pad}.{low:08X} = {}", summ(fv));
                if depth > 0 && matches!(fv, GomValue::Embedded(_) | GomValue::List(_) | GomValue::Map(_))
                {
                    dump(fv, depth - 1, indent + 1);
                }
            }
        }
        GomValue::List(items) => {
            for (i, it) in items.iter().enumerate() {
                if i >= 40 {
                    println!("{pad}... ({} more)", items.len() - 40);
                    break;
                }
                println!("{pad}[{i}] = {}", summ(it));
                if depth > 0 && matches!(it, GomValue::Embedded(_) | GomValue::List(_) | GomValue::Map(_)) {
                    dump(it, depth - 1, indent + 1);
                }
            }
        }
        GomValue::Map(m) => {
            for (i, (k, val)) in m.iter().enumerate() {
                if i >= 40 {
                    println!("{pad}... ({} more)", m.len() - 40);
                    break;
                }
                println!("{pad}{} => {}", summ(k), summ(val));
                if depth > 0 && matches!(val, GomValue::Embedded(_) | GomValue::List(_) | GomValue::Map(_)) {
                    dump(val, depth - 1, indent + 1);
                }
            }
        }
        _ => println!("{pad}{}", summ(v)),
    }
}

/// Find all field-id low32 fingerprints present in an embedded object (sorted).
fn field_ids(v: &GomValue) -> Vec<u32> {
    match v {
        GomValue::Embedded(f) => {
            let mut ids: Vec<u32> = f.iter().map(|(id, _)| *id as u32).collect();
            ids.sort_unstable();
            ids
        }
        _ => vec![],
    }
}

/// Count absolute 5CE87488 markers as a hint for the expected node count.
fn node_count_hint(payload: &[u8]) -> usize {
    let m: [u8; 9] = [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
    (0..payload.len().saturating_sub(9))
        .filter(|&w| payload[w..w + 9] == m)
        .count()
}

fn hexdump(bytes: &[u8], base: usize) {
    for i in (0..bytes.len()).step_by(16) {
        let chunk = &bytes[i..(i + 16).min(bytes.len())];
        let hex: String = chunk.iter().map(|b| format!("{b:02X} ")).collect();
        let asc: String = chunk
            .iter()
            .map(|&b| if (32..127).contains(&b) { b as char } else { '.' })
            .collect();
        println!("  {:06x}  {hex:<48}  {asc}", base + i);
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut hash_path: Option<PathBuf> = None;
    let mut targets: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-H" => {
                hash_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            other => {
                targets.push(other.to_string());
                i += 1;
            }
        }
    }
    let hash_path = hash_path.unwrap_or_else(|| input_dir.join("hashes_filename.txt"));
    let mut hashes = HashDictionary::new();
    hashes.load(&hash_path)?;
    let proto_hashes: HashSet<u64> = hashes
        .paths_matching("/resources/systemgenerated/prototypes/")
        .into_iter()
        .map(|(h, _)| h)
        .collect();
    let target_set: HashSet<String> = targets.iter().cloned().collect();

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut found: HashSet<String> = HashSet::new();
    for tor_path in &tor_files {
        if found.len() == target_set.len() {
            break;
        }
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if !proto_hashes.contains(&entry.filename_hash) || entry.compressed_size == 0 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < 0x18 || &data[0..4] != b"PROT" {
                continue;
            }
            let mut e = 0x14;
            while e < data.len() && data[e] != 0 {
                e += 1;
            }
            let node_fqn = String::from_utf8_lossy(&data[0x14..e]).into_owned();
            if !target_set.contains(&node_fqn) || found.contains(&node_fqn) {
                continue;
            }
            found.insert(node_fqn.clone());
            let path = hashes.get(entry.filename_hash).cloned().unwrap_or_default();

            println!("\n\n############################################################");
            println!("# {node_fqn}");
            println!("#   path={path}  payload={} bytes (PROT total {})", data.len() - e, data.len());
            println!("############################################################");

            let payload = &data[e..];

            // --- raw marker census: every CF40-prefixed field id and its tag ---
            println!("\n--- CF40 field-marker census (offset, then bytes after CF40) ---");
            let mut occ = 0u32;
            let mut marker_offs: Vec<usize> = Vec::new();
            for w in 0..payload.len().saturating_sub(4) {
                if payload[w..w + 4] == FIELD_MARKER {
                    marker_offs.push(w);
                }
            }
            println!("  {} CF40 markers in payload", marker_offs.len());

            // --- 5CE87488 dialogue-line marker census (the prior-research hash) ---
            // The dialogue line value carries '02 cc 11' + line-id u32 BE.
            // Search raw payload for the byte pattern 02 CC 11 and print context.
            println!("\n--- '02 CC 11' line-marker census (line-id follows as u32 BE) ---");
            let mut line_hits = 0u32;
            let mut line_ids: Vec<u32> = Vec::new();
            for w in 0..payload.len().saturating_sub(7) {
                if payload[w] == 0x02 && payload[w + 1] == 0xCC && payload[w + 2] == 0x11 {
                    let id = u32::from_be_bytes([
                        payload[w + 3],
                        payload[w + 4],
                        payload[w + 5],
                        payload[w + 6],
                    ]);
                    line_ids.push(id);
                    if line_hits < 30 {
                        println!("  @0x{w:05x}: line_id={id} (0x{id:08X})  ctx={:02X?}", &payload[w.saturating_sub(8)..(w + 12).min(payload.len())]);
                    }
                    line_hits += 1;
                }
            }
            println!("  TOTAL '02 CC 11' hits: {line_hits}");
            println!("  line_ids in payload byte order (first 40): {:?}", &line_ids[..line_ids.len().min(40)]);

            let _ = occ;
            // --- BRUTE-FORCE: find the node LIST (a List whose elements are
            // Embedded objects, each beginning with field 5CE87488). Scan every
            // offset, try read_value(LIST=07); keep the decode that yields the
            // most Embedded elements whose first field id low32 == 5CE87488. ---
            {
                let want = node_count_hint(payload);
                println!("\n--- BRUTE-FORCE node-list search (expect ~{want} node objs) ---");
                let mut best: Option<(usize, GomValue, usize)> = None;
                for off in 0..payload.len().saturating_sub(2) {
                    let mut r = Reader::new(payload, off);
                    if let Ok(v @ GomValue::List(_)) = r.read_value(0x07) {
                        if let GomValue::List(items) = &v {
                            let n = items
                                .iter()
                                .filter(|e| matches!(e, GomValue::Embedded(f) if f.first().map(|(id,_)| *id as u32)==Some(0x5CE87488)))
                                .count();
                            if n >= 1 && best.as_ref().map(|(_, _, bn)| n > *bn).unwrap_or(true) {
                                best = Some((off, v.clone(), n));
                            }
                        }
                    }
                }
                if let Some((off, v, n)) = best {
                    println!("  best node-list @0x{off:05x}: {n} line-node elements");
                    if let GomValue::List(items) = &v {
                        // Dump first 3 node objects fully and any with >1 transition.
                        for (i, e) in items.iter().enumerate().take(3) {
                            println!("  --- node-list element [{i}] ---");
                            dump(e, 6, 3);
                        }
                    }
                } else {
                    println!("  (no node list found via brute force)");
                }
            }

            // --- attempt full single-object decode (read_object_fields) ---
            println!("\n--- FULL OBJECT DECODE attempt (read_object_fields) ---");
            match read_object_fields(payload) {
                Ok(root) => {
                    println!("  OK -- top structure (depth 3):");
                    dump(&root, 3, 2);
                }
                Err(e) => println!("  FAILED: {e}"),
            }

            // --- flat top-level field walk ---
            println!("\n--- TOP-LEVEL FIELD WALK (flat <id><tag><value> sequence) ---");
            let fields = walk_top_fields(payload);
            println!("  decoded {} top-level fields", fields.len());

            // Field-id low32 histogram
            use std::collections::BTreeMap;
            let mut hist: BTreeMap<u32, u32> = BTreeMap::new();
            for (id, _, v) in &fields {
                if v.is_ok() {
                    *hist.entry(*id as u32).or_default() += 1;
                }
            }
            println!("\n--- top-level field-id low32 histogram ---");
            for (id, cnt) in &hist {
                println!("  .{id:08X}  x{cnt}");
            }

            // Ordered field sequence (id low32 + compact value) -- reveals node
            // boundaries (a repeating field id that starts each dialogue node).
            println!("\n--- ORDERED top-level field sequence ---");
            for (i, (id, off, v)) in fields.iter().enumerate() {
                let low = *id as u32;
                let s = match v {
                    Ok(val) => summ(val),
                    Err(e) => format!("ERR {e}"),
                };
                println!("  [{i:>3}] @0x{off:05x} .{low:08X} = {s}");
            }

            // Dump the FIRST 8 fields fully and any node-list fields.
            println!("\n--- first 12 top-level fields (id, offset, value) ---");
            for (id, off, v) in fields.iter().take(12) {
                let low = *id as u32;
                match v {
                    Ok(val) => {
                        println!("  @0x{off:05x} .{low:08X} (full 0x{id:016X}) = {}", summ(val));
                        if matches!(val, GomValue::Embedded(_) | GomValue::List(_) | GomValue::Map(_)) {
                            dump(val, 2, 3);
                        }
                    }
                    Err(e) => println!("  @0x{off:05x} .{low:08X} = ERR {e}"),
                }
            }

            // Characterize node-record lists: any field that is a List of Embedded.
            println!("\n--- node-record fingerprint analysis (lists of embedded objs) ---");
            for (id, _, v) in &fields {
                if let Ok(val) = v {
                    characterize_field(*id as u32, val);
                }
            }

            // --- PER-NODE OBJECT DECODE ---
            // Each dialogue node is an Embedded object whose FIRST field uses the
            // absolute 8-byte id 0x40000011_5CE87488 (the line-id field). Locate
            // each such absolute id in the payload, back up to the object header
            // (<script_type><nfields>), and decode the whole node object so we
            // see the line-id, str.cnv ref, speaker, options and branch links.
            println!("\n--- PER-NODE OBJECT DECODE (first 6 nodes) ---");
            let abs_marker: [u8; 9] =
                [0xCF, 0x40, 0x00, 0x00, 0x11, 0x5C, 0xE8, 0x74, 0x88];
            let mut node_positions: Vec<usize> = Vec::new();
            for w in 0..payload.len().saturating_sub(9) {
                if payload[w..w + 9] == abs_marker {
                    node_positions.push(w);
                }
            }
            println!("  {} node objects (absolute 5CE87488 markers)", node_positions.len());
            let mut decoded_nodes = 0;
            for (ni, &mpos) in node_positions.iter().enumerate() {
                let node_cap: usize = std::env::var("NODE_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
                if decoded_nodes >= node_cap {
                    break;
                }
                // Back up to find the object header: try offsets 1..=10 bytes
                // before the absolute id, decode as object, keep the one whose
                // first field id low32 == 5CE87488.
                let mut best: Option<GomValue> = None;
                for back in 1..=12usize {
                    if mpos < back {
                        continue;
                    }
                    let cand = mpos - back;
                    let mut r = Reader::new(payload, cand);
                    if let Ok(v) = r.read_value(0x09) {
                        if let GomValue::Embedded(f) = &v {
                            if f.first().map(|(id, _)| *id as u32) == Some(0x5CE87488) {
                                best = Some(v);
                                break;
                            }
                        }
                    }
                }
                println!("\n  === NODE {ni} @0x{mpos:05x} ===");
                if let Some(v) = best {
                    dump(&v, 5, 4);
                }
                // Manual delta-field walk from the line marker to the next node:
                // start a Reader at the absolute id, treat it as the running
                // field-id, and decode <tag><value> then <delta_id><tag><value>*
                // until the next node position (or a decode error).
                let end = node_positions.get(ni + 1).copied().unwrap_or(payload.len());
                println!("    [manual delta-field walk to next node @0x{end:05x}]");
                let mut r = Reader::new(payload, mpos);
                let mut fid: u64 = match r.read_number() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // first field id is the absolute value just read
                loop {
                    let tag = match r.read_tag() {
                        Ok(t) => t,
                        Err(_) => break,
                    };
                    let off = r.pos();
                    let val = match r.read_value(tag) {
                        Ok(v) => v,
                        Err(e) => {
                            println!("      .{:08X} tag={tag:02X} DECODE-ERR {e} @0x{off:05x}", fid as u32);
                            break;
                        }
                    };
                    println!("      .{:08X} (tag {tag:02X}) = {}", fid as u32, summ(&val));
                    if matches!(val, GomValue::List(_) | GomValue::Map(_) | GomValue::Embedded(_)) {
                        dump(&val, 3, 5);
                    }
                    if r.pos() >= end {
                        break;
                    }
                    // next field: read delta id
                    let d = match r.read_number() {
                        Ok(v) => v,
                        Err(_) => break,
                    };
                    fid = fid.wrapping_add(d);
                }
                // Per-node CF E0 GUID census (speaker/branch refs) within this
                // node's byte range, and the node-type enum F03749AA.
                let e0: Vec<String> = (mpos..end.saturating_sub(8))
                    .filter(|&w| payload[w] == 0xCF && payload[w + 1] == 0xE0)
                    .map(|w| {
                        let g = u64::from_be_bytes(payload[w + 1..w + 9].try_into().unwrap());
                        format!("{g:016X}")
                    })
                    .collect();
                println!("    [node E0 GUID refs: {e0:?}]");
                // node-type enum F03749AA: raw bytes 05 <val+1>. Search the node
                // for the absolute field marker cf40 0005 F03749AA.
                // (Simpler: report whether the 9EB734DC option-fingerprint field
                // appears in this node's range, and any ASCII transition labels.)
                let has_9eb = (mpos..end.saturating_sub(4)).any(|w| {
                    payload[w] == 0x9E && payload[w + 1] == 0xB7 && payload[w + 2] == 0x34 && payload[w + 3] == 0xDC
                });
                // transition label strings (06 <len> ascii) that look like To_*/branch labels
                let mut labels: Vec<String> = Vec::new();
                let mut w = mpos;
                while w + 2 < end {
                    if payload[w] == 0x06 {
                        let len = payload[w + 1] as usize;
                        if len > 2 && len < 60 && w + 2 + len <= end {
                            let s = &payload[w + 2..w + 2 + len];
                            if s.iter().all(|&b| (0x20..0x7F).contains(&b)) {
                                let st = String::from_utf8_lossy(s).into_owned();
                                if !st.starts_with("str.") && !labels.contains(&st) {
                                    labels.push(st);
                                }
                            }
                        }
                    }
                    w += 1;
                }
                println!("    [node has_9EB734DC={has_9eb}  labels={labels:?}]");
                decoded_nodes += 1;
            }

            // The dialogue line node: find which top-level field (or nested obj)
            // contains the .5CE87488 sub-field, and dump 3 full samples plus the
            // node objects that surround line markers.
            println!("\n--- LINE-NODE samples (objects containing .5CE87488) ---");
            let mut shown = 0;
            for (id, off, v) in &fields {
                if let Ok(val) = v {
                    if contains_field(val, 0x5CE87488) && shown < 4 {
                        println!("  >>> top-field @0x{off:05x} .{:08X} contains a 5CE87488 line node:", *id as u32);
                        dump(val, 4, 3);
                        shown += 1;
                    }
                }
            }
        }
    }

    for t in &targets {
        if !found.contains(t) {
            println!("\n!!! NOT FOUND: {t}");
        }
    }
    Ok(())
}

/// True if this value or any nested object/list/map contains an embedded field
/// whose id low32 == `want`.
fn contains_field(v: &GomValue, want: u32) -> bool {
    match v {
        GomValue::Embedded(fields) => fields
            .iter()
            .any(|(id, fv)| (*id as u32) == want || contains_field(fv, want)),
        GomValue::List(items) => items.iter().any(|x| contains_field(x, want)),
        GomValue::Map(m) => m.iter().any(|(_, val)| contains_field(val, want)),
        _ => false,
    }
}

/// For a single top-level field value, if it is (or nests) a List of Embedded
/// objects, report distinct field-id fingerprints with counts + one sample.
fn characterize_field(top_id: u32, v: &GomValue) {
    if let GomValue::List(items) = v {
        let emb: Vec<&GomValue> = items
            .iter()
            .filter(|x| matches!(x, GomValue::Embedded(_)))
            .collect();
        if emb.len() >= 2 {
            println!("  top-field .{top_id:08X} -> list of {} embedded objects", emb.len());
            let mut seen: Vec<(Vec<u32>, usize)> = Vec::new();
            for e in &emb {
                let fp = field_ids(e);
                if let Some(s) = seen.iter_mut().find(|(f, _)| *f == fp) {
                    s.1 += 1;
                } else {
                    seen.push((fp, 1));
                }
            }
            for (fp, cnt) in &seen {
                let fps: Vec<String> = fp.iter().map(|x| format!("{x:08X}")).collect();
                println!("      {cnt:>4}x fingerprint [{}]", fps.join(" "));
            }
        }
    }
}

/// Walk the tree; for every List whose elements are Embedded objects, tally the
/// distinct field-id fingerprints (the node-type fingerprints).
#[allow(dead_code)]
fn characterize(v: &GomValue, depth: usize) {
    match v {
        GomValue::Embedded(fields) => {
            for (id, fv) in fields {
                if let GomValue::List(items) = fv {
                    let emb: Vec<&GomValue> =
                        items.iter().filter(|x| matches!(x, GomValue::Embedded(_))).collect();
                    if !emb.is_empty() {
                        let low = *id as u32;
                        println!("  field .{low:08X} -> list of {} embedded objects", emb.len());
                        // distinct fingerprints
                        let mut seen: Vec<(Vec<u32>, usize)> = Vec::new();
                        for e in &emb {
                            let fp = field_ids(e);
                            if let Some(s) = seen.iter_mut().find(|(f, _)| *f == fp) {
                                s.1 += 1;
                            } else {
                                seen.push((fp, 1));
                            }
                        }
                        for (fp, cnt) in &seen {
                            let fps: Vec<String> = fp.iter().map(|x| format!("{x:08X}")).collect();
                            println!("      {cnt:>4}x fingerprint [{}]", fps.join(" "));
                        }
                        // dump first 2 of each fingerprint fully
                        for (fp, _) in &seen {
                            if let Some(sample) = emb.iter().find(|e| &field_ids(e) == fp) {
                                println!("      --- sample for fingerprint [{}] ---", fp.iter().map(|x| format!("{x:08X}")).collect::<Vec<_>>().join(" "));
                                dump(sample, 2, 4);
                            }
                        }
                    }
                }
                if depth < 4 {
                    characterize(fv, depth + 1);
                }
            }
        }
        GomValue::List(items) => {
            for it in items.iter().take(3) {
                if depth < 4 {
                    characterize(it, depth + 1);
                }
            }
        }
        _ => {}
    }
}
