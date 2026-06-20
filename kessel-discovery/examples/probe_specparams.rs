//! Locate the SpecParam list in an ability payload: the array the `<<N>>`
//! description tokens index. Oracle: abl.agent.evasion reads "by 200% for
//! <<1>> seconds" -- so the payload must contain 200 and 3 (its in-game
//! duration) somewhere structured. Dumps the GOM field tree and flags those.
//!
//! Read-only. cargo run -p kessel-discovery --example probe_specparams

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_reader::{read_object_fields, GomValue};
use rusqlite::{Connection, OpenFlags};

const DB: &str = "/Users/seansilvius/swtor/data/spice-7.9.a-v7.sqlite";
const FQNS: &[&str] = &[
    "abl.agent.evasion",
    "abl.agent.corrosive_dart",
    "abl.agent.diagnostic_scan",
    "abl.sith_warrior.enrage",
];

/// SpecParam list field + per-entry value field (located via the evasion oracle).
const SPECPARAM_LIST: u32 = 0x384b_793a;

fn main() -> Result<()> {
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    for fqn in FQNS {
        let b64: Option<String> = conn
            .query_row(
                "SELECT json_extract(json,'$.payload_b64') FROM objects WHERE fqn=?1 AND is_canonical=1",
                [fqn],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let Some(b64) = b64 else {
            println!("== {fqn}: no payload ==");
            continue;
        };
        let raw: Option<String> = conn
            .query_row(
                "SELECT s.text_raw FROM objects o JOIN strings s ON s.id2=o.string_id \
                 AND s.id1=1 AND s.locale='en-us' WHERE o.fqn=?1 AND o.is_canonical=1",
                [fqn],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let payload = BASE64.decode(&b64)?;
        println!("\n== {fqn} ==");
        println!("  text_raw: {}", raw.as_deref().unwrap_or("(none)"));
        match read_object_fields(&payload) {
            Ok(obj) => {
                if let Some(list) = obj
                    .embedded_field(SPECPARAM_LIST)
                    .and_then(GomValue::as_list)
                {
                    for (i, entry) in list.iter().enumerate() {
                        println!("  SpecParam[{i}] (<<{}>>):", i + 1);
                        if let GomValue::Embedded(fields) = entry {
                            for (id, v) in fields {
                                println!("      field {:08x} = {v:?}", *id as u32);
                            }
                        } else {
                            println!("      {entry:?}");
                        }
                    }
                } else {
                    println!("  (no SpecParam list field {SPECPARAM_LIST:08x})");
                }
            }
            Err(e) => println!("  decode err: {e}"),
        }
    }
    Ok(())
}
