//! Verify populate_gsf_ability_stats against an existing spice.sqlite.
//!
//! Usage: ./target/release/verify_gsf_ability_stats <path-to-spice.sqlite>
//!
//! Runs the populate fn against an already-extracted spice db, then prints
//! coverage totals and validates the documented anchors from #78.

use anyhow::Result;
use kessel::db::Database;
use rusqlite::Connection;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: verify_gsf_ability_stats <spice.sqlite>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let db = Database::with_grammar(&path, None)?;
    db.init_schema()?;

    let count = db.populate_gsf_ability_stats()?;
    println!("populate_gsf_ability_stats inserted {count} rows");

    drop(db);
    let conn = Connection::open(&path)?;

    let with_records: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT ability_game_id) FROM gsf_ability_stats",
        [],
        |r| r.get(0),
    )?;
    let total_abilities: i64 = conn.query_row(
        "SELECT COUNT(*) FROM objects WHERE fqn LIKE 'abl.spvp.%'",
        [],
        |r| r.get(0),
    )?;
    println!("abilities with records: {with_records} / {total_abilities}");

    let unique_prop_ids: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT prop_id) FROM gsf_ability_stats",
        [],
        |r| r.get(0),
    )?;
    println!("unique prop_ids: {unique_prop_ids}");

    println!("\ntop 10 most-frequent prop IDs:");
    let mut stmt = conn.prepare(
        "SELECT prop_id, COUNT(*) c, MIN(value), MAX(value) \
         FROM gsf_ability_stats GROUP BY prop_id ORDER BY c DESC LIMIT 10",
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
        let (pid, c, mn, mx) = row;
        println!("  0x{pid:04x}  count={c:<4}  range=[{mn:.3}, {mx:.3}]");
    }

    println!("\ndocumented anchors (issue #78):");
    let samples = [
        ("abl.spvp.engine.barrel_roll", "0x0402 expected = 30.0"),
        (
            "abl.spvp.engine.power_dive",
            "0x0402 expected = 15.0 (inferred)",
        ),
    ];
    for (fqn, hint) in samples {
        let mut stmt = conn.prepare(
            "SELECT prop_id, value FROM gsf_ability_stats \
             WHERE ability_game_id = (SELECT game_id FROM objects WHERE fqn=?) \
             ORDER BY ordinal",
        )?;
        let rows: Vec<(i64, f64)> = stmt
            .query_map([fqn], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        println!("  {fqn} ({hint}):");
        for (pid, val) in rows {
            println!("    -> prop=0x{pid:04x} val={val}");
        }
    }

    Ok(())
}
