//! Oracle check for the item itemization decode. Opens the canonical v22 DB
//! read-only, pulls the three prototype singletons, decodes them with the
//! production `kessel::gom_reader`, and asserts the known truths:
//!   - budget[artifact][89] contains 484 and 167 (a real artifact relic)
//!   - rating and modifier-package tables are non-empty and well-shaped
//!
//! Read-only: never writes to the DB, never copies it. Run with:
//!   cargo run -p kessel-discovery --bin verify_item_itemization

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_reader::{read_first_field, GomValue};
use rusqlite::{Connection, OpenFlags};

const DB: &str = "/Users/seansilvius/swtor/data/spice.sqlite";

fn singleton(conn: &Connection, fqn: &str) -> Result<Vec<u8>> {
    let b64: String = conn
        .query_row(
            "SELECT payload_b64 FROM singletons WHERE fqn = ?1",
            [fqn],
            |r| r.get(0),
        )
        .with_context(|| format!("singleton {fqn} not found"))?;
    Ok(BASE64.decode(b64)?)
}

fn quality_name(idx: i64) -> Option<String> {
    let e = kessel::gom_schema::enum_for_name("itmQuality")?;
    let m = e.members.get(usize::try_from(idx).ok()?)?;
    Some(m.strip_prefix("itmQuality").unwrap_or(m).to_ascii_lowercase())
}

fn stat_name(idx: i64) -> Option<String> {
    let e = kessel::gom_schema::enum_for_name("STAT")?;
    e.members.get(usize::try_from(idx).ok()?).cloned()
}

fn main() -> Result<()> {
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    // -- Rating --
    let rating = read_first_field(&singleton(&conn, "itmRatingTablePrototype")?)?;
    let mut rating_rows = 0usize;
    for (_lvl, inner) in rating.as_map().unwrap_or(&[]) {
        rating_rows += inner.as_map().map_or(0, |m| m.len());
    }
    println!("rating rows: {rating_rows}");

    // -- Budget: find artifact, level 89, scan for 484 and 167 --
    let budget = read_first_field(&singleton(&conn, "itmBudgetedAttributesPrototype")?)?;
    let mut budget_rows = 0usize;
    let mut has_484 = false;
    let mut has_167 = false;
    for (q_key, levels) in budget.as_map().unwrap_or(&[]) {
        let (Some(q), Some(level_list)) = (q_key.as_i64(), levels.as_list()) else {
            continue;
        };
        let qname = quality_name(q).unwrap_or_default();
        for (level, slots) in level_list.iter().enumerate() {
            let slot_list = slots.as_list().unwrap_or(&[]);
            budget_rows += slot_list.len();
            if qname == "artifact" && level == 89 {
                for v in slot_list {
                    match v.as_i64() {
                        Some(484) => has_484 = true,
                        Some(167) => has_167 = true,
                        _ => {}
                    }
                }
            }
        }
    }
    println!("budget rows: {budget_rows}");
    println!("budget[artifact][89] has 484: {has_484}, has 167: {has_167}");

    // -- Modifier packages: count splits, print one example --
    let modpkg = read_first_field(&singleton(&conn, "itmModifierPackageTablePrototype")?)?;
    let mut modpkg_rows = 0usize;
    let mut example: Option<String> = None;
    for (mod_key, pkg_val) in modpkg.as_map().unwrap_or(&[]) {
        let Some(mod_id) = mod_key.as_i64() else {
            continue;
        };
        let pkg = match pkg_val {
            GomValue::List(items) => items.first(),
            other @ GomValue::Embedded(_) => Some(other),
            _ => None,
        };
        let Some(pct_map) = pkg
            .and_then(GomValue::embedded_first_map)
            .and_then(GomValue::as_map)
        else {
            continue;
        };
        let mut split: Vec<String> = Vec::new();
        for (stat_key, pct_val) in pct_map {
            let (Some(si), Some(p)) = (stat_key.as_i64(), pct_val.as_i64()) else {
                continue;
            };
            modpkg_rows += 1;
            if p > 0 {
                split.push(format!("{}={}", stat_name(si).unwrap_or_default(), p));
            }
        }
        if example.is_none() && split.len() >= 2 {
            example = Some(format!("mod {mod_id}: {}", split.join(" + ")));
        }
    }
    println!("modifier-package rows: {modpkg_rows}");
    if let Some(ex) = example {
        println!("example split: {ex}");
    }

    let ok = rating_rows > 0 && budget_rows > 0 && modpkg_rows > 0 && has_484 && has_167;
    println!("\nVERDICT: {}", if ok { "WORKS" } else { "FAILED" });
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
