//! kessel-compare: the SWTOR patch changelog.
//!
//! Diffs two kessel spice databases (old patch vs new) and reports what changed
//! per object kind -- new quests, retuned abilities/talents, added items, etc.
//!
//! Why a custom tool instead of `smugglr diff`: every kessel table's primary key
//! embeds `game_id = sha256(fqn:guid)`, and the GUID *shifts on every patch* -- so a
//! PK-keyed diff would report every object as removed+added. kessel carries the
//! cross-patch primitives for exactly this: `stable_id = sha256(fqn)` (survives a
//! patch) and `payload_hash = sha256(payload)` (the content-change signal). We feed
//! smugglr's pure `classify_diff` a metadata map keyed by `stable_id` with
//! `content_hash = payload_hash`, so its content-hash engine partitions correctly:
//!   - in new, not old        -> ADDED
//!   - in old, not new        -> REMOVED
//!   - same stable_id, payload differs -> CHANGED
//!
//! Usage: kessel-compare <old.sqlite> <new.sqlite>

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use smugglr_core::datasource::RowMeta;
use smugglr_core::diff::classify_diff;

struct ObjInfo {
    kind: String,
    fqn: String,
}

/// Load every canonical object as (stable_id -> content metadata) for smugglr's
/// diff, plus (stable_id -> kind/fqn) for reporting.
fn load(path: &str) -> Result<(HashMap<String, RowMeta>, HashMap<String, ObjInfo>)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {path}"))?;
    let mut stmt = conn.prepare(
        "SELECT stable_id, kind, fqn, payload_hash FROM objects WHERE is_canonical = 1",
    )?;
    let mut meta = HashMap::new();
    let mut info = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (stable_id, kind, fqn, payload_hash) = row?;
        meta.insert(
            stable_id.clone(),
            RowMeta {
                pk_value: stable_id.clone(),
                updated_at: None, // no timestamps in spice -> content-hash decides
                content_hash: payload_hash,
            },
        );
        info.insert(stable_id, ObjInfo { kind, fqn });
    }
    Ok((meta, info))
}

#[derive(Default)]
struct KindDelta {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: kessel-compare <old.sqlite> <new.sqlite>");
        std::process::exit(2);
    }
    let (old_meta, old_info) = load(&args[1])?;
    let (new_meta, new_info) = load(&args[2])?;

    // local = new, remote = old. smugglr's pure classifier, keyed by stable_id.
    let diff = classify_diff(&new_meta, &old_meta, "objects");

    let mut by_kind: BTreeMap<String, KindDelta> = BTreeMap::new();
    for sid in &diff.local_only {
        if let Some(o) = new_info.get(sid) {
            by_kind.entry(o.kind.clone()).or_default().added.push(o.fqn.clone());
        }
    }
    for sid in &diff.content_differs {
        if let Some(o) = new_info.get(sid) {
            by_kind.entry(o.kind.clone()).or_default().changed.push(o.fqn.clone());
        }
    }
    for sid in &diff.remote_only {
        if let Some(o) = old_info.get(sid) {
            by_kind.entry(o.kind.clone()).or_default().removed.push(o.fqn.clone());
        }
    }

    println!("kessel patch diff: {} -> {}", args[1], args[2]);
    println!(
        "objects: {} old, {} new  |  +{} added  ~{} changed  -{} removed\n",
        old_meta.len(),
        new_meta.len(),
        diff.local_only.len(),
        diff.content_differs.len(),
        diff.remote_only.len(),
    );

    for (kind, d) in &by_kind {
        if d.added.is_empty() && d.changed.is_empty() && d.removed.is_empty() {
            continue;
        }
        println!(
            "== {kind} ==  +{} added  ~{} changed  -{} removed",
            d.added.len(),
            d.changed.len(),
            d.removed.len()
        );
        print_sample("added", &d.added);
        print_sample("changed", &d.changed);
        print_sample("removed", &d.removed);
        println!();
    }
    Ok(())
}

/// Print up to 15 fqns for a bucket (full lists get long on a big patch).
fn print_sample(label: &str, fqns: &[String]) {
    if fqns.is_empty() {
        return;
    }
    let mut sorted = fqns.to_vec();
    sorted.sort();
    let shown = sorted.len().min(15);
    println!("   {label} ({}):", fqns.len());
    for f in &sorted[..shown] {
        println!("     {f}");
    }
    if fqns.len() > shown {
        println!("     ... and {} more", fqns.len() - shown);
    }
}
