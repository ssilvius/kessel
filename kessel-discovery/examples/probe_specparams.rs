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

const DB: &str = "/Users/seansilvius/swtor/data/spice-7.9.a-v5.sqlite";
const FQNS: &[&str] = &[
    "abl.agent.evasion",
    "abl.agent.corrosive_dart",
    "abl.agent.diagnostic_scan",
    "abl.sith_warrior.enrage",
];

/// SpecParam list field + per-entry value field (located via the evasion oracle).
const SPECPARAM_LIST: u32 = 0x384b_793a;
const SPECPARAM_VALUE: u32 = 0x384b_7939;

fn dump(v: &GomValue, path: &str, depth: usize) {
    if depth > 6 {
        return;
    }
    match v {
        GomValue::Embedded(fields) => {
            for (id, val) in fields {
                let p = format!("{path}.{:08x}", *id as u32);
                dump(val, &p, depth + 1);
            }
        }
        GomValue::List(items) => {
            for (i, item) in items.iter().enumerate() {
                dump(item, &format!("{path}[{i}]"), depth + 1);
            }
        }
        GomValue::Map(entries) => {
            for (k, val) in entries {
                dump(val, &format!("{path}{{{k:?}}}"), depth + 1);
            }
        }
        GomValue::F32(f) => {
            let flag = if (*f - 3.0).abs() < 0.01 || (*f - 200.0).abs() < 0.01 {
                "  <== ORACLE"
            } else {
                ""
            };
            println!("  {path} = f32 {f}{flag}");
        }
        GomValue::I64(n) | GomValue::Enum(n) => {
            let flag = if *n == 3 || *n == 200 { "  <== ORACLE" } else { "" };
            println!("  {path} = int {n}{flag}");
        }
        GomValue::U64(n) => {
            let flag = if *n == 3 || *n == 200 { "  <== ORACLE" } else { "" };
            println!("  {path} = u64 {n}{flag}");
        }
        _ => {}
    }
}

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
                if let Some(list) = obj.embedded_field(SPECPARAM_LIST).and_then(GomValue::as_list) {
                    for (i, entry) in list.iter().enumerate() {
                        let val = entry.embedded_field(SPECPARAM_VALUE);
                        println!("  SpecParam[{i}] (<<{}>>) = {val:?}", i + 1);
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

#[allow(dead_code)]
fn unused() {}
