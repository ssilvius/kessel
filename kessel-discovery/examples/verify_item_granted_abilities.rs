//! Coverage + oracle check for the item granted-ability decode. Opens v23
//! read-only, walks every canonical itm.* payload with the production
//! `kessel::gom_reader`, extracts the granted-ability guid (field 0x2d7b8786),
//! and reports: how many items grant an ability, decode-failure rate, how many
//! resolve to an extracted ability, and the Fearless Victor oracle.
//!
//! Read-only. Run with:
//!   cargo run -p kessel-discovery --bin verify_item_granted_abilities

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_reader::{read_object_fields, GomValue};
use rusqlite::{Connection, OpenFlags};

const DB: &str = "/Users/seansilvius/swtor/data/spice.sqlite";
const GRANTED_ABILITY_FIELD: u32 = 0x2d7b_8786;

fn main() -> Result<()> {
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut select = conn.prepare(
        "SELECT fqn, json_extract(json, '$.payload_b64') \
         FROM objects WHERE fqn LIKE 'itm.%' AND is_canonical = 1",
    )?;
    let rows = select
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let (mut total, mut decode_fail, mut has_field, mut resolved) = (0u64, 0u64, 0u64, 0u64);
    let mut fv_effect: Option<String> = None;

    let mut resolve = conn.prepare(
        "SELECT o.fqn, s.text FROM objects o \
         LEFT JOIN strings s ON s.id2=o.string_id AND s.id1=1 AND s.locale='en-us' \
         WHERE o.guid=?1 AND o.is_canonical=1 LIMIT 1",
    )?;

    for (fqn, b64) in &rows {
        total += 1;
        let Some(b64) = b64 else { continue };
        let Ok(payload) = BASE64.decode(b64) else {
            continue;
        };
        let obj = match read_object_fields(&payload) {
            Ok(o) => o,
            Err(_) => {
                decode_fail += 1;
                continue;
            }
        };
        let Some(guid) = obj
            .embedded_field(GRANTED_ABILITY_FIELD)
            .and_then(GomValue::as_i64)
        else {
            continue;
        };
        has_field += 1;
        let guid_hex = format!("{:016X}", guid as u64);
        let hit = resolve
            .query_row([&guid_hex], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            })
            .ok();
        if let Some((Some(afqn), text)) = hit {
            resolved += 1;
            if fqn == "itm.legendary.kni_war.ilvl_0154.fearless_victor" {
                fv_effect = Some(format!("{afqn} => {}", text.unwrap_or_default()));
            }
        }
    }

    println!("itm.* canonical objects:      {total}");
    println!("  decode failures (skipped):  {decode_fail}");
    println!("  grant an ability (field):   {has_field}");
    println!("  resolved to extracted abl:  {resolved}");
    println!("  unresolved (proc objects):  {}", has_field - resolved);
    println!("\nFearless Victor oracle:");
    println!(
        "  {}",
        fv_effect.clone().unwrap_or_else(|| "(NOT FOUND)".into())
    );

    let ok = has_field > 0
        && resolved > 0
        && fv_effect
            .as_deref()
            .is_some_and(|s| s.contains("melee damage is increased"));
    println!("\nVERDICT: {}", if ok { "WORKS" } else { "FAILED" });
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
