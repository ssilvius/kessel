//! Verify populate_gsf_talent_stats against an existing spice.sqlite.
//!
//! Usage: ./target/release/verify_gsf_talent_stats <path-to-spice.sqlite>
//!
//! Runs the populate fn against a copy of an already-extracted spice db
//! (which already has objects + json + payload_b64), then prints record
//! totals and a sample of decoded stats. Used to sanity-check the new
//! decoder against real game data without re-running the full extraction.

use anyhow::Result;
use kessel::db::Database;
use rusqlite::Connection;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: verify_gsf_talent_stats <spice.sqlite>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let db = Database::with_grammar(&path, None)?;
    db.init_schema()?;

    let count = db.populate_gsf_talent_stats()?;
    println!("populate_gsf_talent_stats inserted {count} rows");

    drop(db);
    let conn = Connection::open(&path)?;

    let with_records: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT talent_game_id) FROM gsf_talent_stats",
        [],
        |r| r.get(0),
    )?;
    let total_talents: i64 = conn.query_row(
        "SELECT COUNT(*) FROM objects WHERE fqn LIKE 'tal.spvp.%'",
        [],
        |r| r.get(0),
    )?;
    println!("talents with records: {with_records} / {total_talents}");

    let unique_stat_ids: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT stat_id) FROM gsf_talent_stats",
        [],
        |r| r.get(0),
    )?;
    println!("unique stat_ids: {unique_stat_ids}");

    println!("\ntop 10 most-frequent stat IDs:");
    let mut stmt = conn.prepare(
        "SELECT stat_id, COUNT(*) c, MIN(value), MAX(value) \
         FROM gsf_talent_stats GROUP BY stat_id ORDER BY c DESC LIMIT 10",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, f64>(3)?,
        ))
    })?;
    for row in rows.flatten() {
        let (sid, c, mn, mx) = row;
        println!("  0x{sid:02x}  count={c:<4}  range=[{mn:.3}, {mx:.3}]");
    }

    println!("\nsample validations:");
    let samples = [
        (
            "tal.spvp.crew.offensive.firing_arc",
            "expects stat 0x5f = 2.0",
        ),
        (
            "tal.spvp.shield.shield_projector.tier1",
            "expects 0x40 = -10.0",
        ),
        ("tal.spvp.engine.tensor_field.tier3", "expects 0x41 = 4.0"),
    ];
    for (fqn, hint) in samples {
        let mut stmt = conn.prepare(
            "SELECT stat_id, value FROM gsf_talent_stats \
             WHERE talent_game_id = (SELECT game_id FROM objects WHERE fqn=?) \
             ORDER BY ordinal",
        )?;
        let rows: Vec<(i64, f64)> = stmt
            .query_map([fqn], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        println!("  {fqn} ({hint}):");
        for (sid, val) in rows {
            println!("    -> stat=0x{sid:02x} val={val}");
        }
    }

    Ok(())
}
