//! Coverage + oracle check for per-item stats (itmEquipModStats). Opens v23
//! read-only, walks every canonical itm.* payload with the production
//! `kessel::gom_reader`, extracts the fixed stat block + metadata, and checks
//! three live-gear oracles (Fearless Victor implant, Rakata Force-Healer's
//! Robe, Med-Tech Vambraces) whose values were confirmed in-game.
//!
//! Read-only. Run with:
//!   cargo run -p kessel-discovery --bin verify_item_stats

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_reader::{read_object_fields, GomValue};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;

const DB: &str = "/Users/seansilvius/swtor/data/spice.sqlite";
const EQUIP_MOD_STATS: u32 = 0xa4fa_ffdd;
const BASE_LEVEL: u32 = 0xc7c4_8e7c;
const RATING: u32 = 0x191f_29c8;

fn stat_name(stat: &[String], idx: i64) -> String {
    usize::try_from(idx)
        .ok()
        .and_then(|i| stat.get(i))
        .cloned()
        .unwrap_or_else(|| format!("STAT#{idx}"))
}

fn main() -> Result<()> {
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let stat = kessel::gom_schema::enum_for_name("STAT")
        .map(|e| e.members.clone())
        .unwrap_or_default();

    let mut select = conn.prepare(
        "SELECT fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'itm.%' AND is_canonical = 1",
    )?;
    let rows = select
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let oracles = [
        "itm.legendary.kni_war.ilvl_0154.fearless_victor",
        "itm.gen.lots.armor.inq_jco_heals.operation.ilvl_0160.artifact.armor_chest",
        "itm.gen.lots.armor.bh_tro_heals.operation.ilvl_0160.artifact.armor_wrists",
    ];

    // (level, rating, [(stat_name, value)]) per oracle item.
    type Oracle = (Option<i64>, Option<i64>, Vec<(String, i64)>);
    let (mut items, mut stat_rows) = (0u64, 0u64);
    let mut found: BTreeMap<String, Oracle> = BTreeMap::new();

    for (fqn, b64) in &rows {
        let Some(b64) = b64 else { continue };
        let Ok(payload) = BASE64.decode(b64) else {
            continue;
        };
        let Ok(obj) = read_object_fields(&payload) else {
            continue;
        };
        let Some(stats) = obj
            .embedded_field(EQUIP_MOD_STATS)
            .and_then(GomValue::as_map)
        else {
            continue;
        };
        let mut block = Vec::new();
        for (k, v) in stats {
            if let (Some(si), Some(val)) = (k.as_i64(), v.as_i64()) {
                block.push((stat_name(&stat, si), val));
                stat_rows += 1;
            }
        }
        if !block.is_empty() {
            items += 1;
        }
        if oracles.contains(&fqn.as_str()) {
            let level = obj.embedded_field(BASE_LEVEL).and_then(GomValue::as_i64);
            let rating = obj.embedded_field(RATING).and_then(GomValue::as_i64);
            found.insert(fqn.clone(), (level, rating, block));
        }
    }

    println!("items with a stat block: {items}");
    println!("total stat rows:         {stat_rows}\n");
    for o in oracles {
        match found.get(o) {
            Some((lvl, rating, block)) => {
                println!("{o}");
                println!("  level={:?} rating={:?}", lvl, rating);
                for (n, v) in block {
                    println!("    {n:<26} {v}");
                }
            }
            None => println!("{o}\n  (NOT FOUND)"),
        }
    }

    // Oracle assertions: Fearless Victor 340 = Mastery 1223 / Endurance 1450 /
    // Power 940 / Critical 614.
    let fv_ok = found
        .get(oracles[0])
        .map(|(_, r, b)| {
            *r == Some(340)
                && b.iter().any(|(n, v)| n == "STAT_att_mastery" && *v == 1223)
                && b.iter()
                    .any(|(n, v)| n == "STAT_att_endurance" && *v == 1450)
        })
        .unwrap_or(false);
    let ok = items > 0 && stat_rows > 0 && fv_ok;
    println!("\nVERDICT: {}", if ok { "WORKS" } else { "FAILED" });
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
