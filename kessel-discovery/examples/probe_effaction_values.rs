//! Trace where an ability's <<N>> base VALUES live (#320). The SpecParam list
//! holds only type+coefficient (#309); the displayed value = coefficient x
//! effAction-base. Oracle: abl.agent.evasion = "+200% dodge for <<1>> seconds"
//! (3s in-game). Dumps the full GOM field tree -- flagging 3/200/2.0 and every
//! ref -- to find the base-value source.
//!
//! FINDING (the answer to #320): the value is NOT recoverable GOM data for the
//! buff/duration cases. evasion's 3 is in NO payload -- not the canonical block
//! (depth-16 dump), not any of its 3 sibling effect-block objects (game_ids
//! below), and there is no effParam_Duration in the vocabulary at all. evasion
//! carries effInitializer_SetHydraScript: its buff magnitude+duration are
//! applied by COMPILED HYDRA SCRIPT, not stored as data. Data-driven abilities
//! (corrosive_dart) do carry in-payload f32s, so recovery is partial and
//! ability-specific; script-driven buffs (evasion, relic procs) are a genuine
//! ceiling. ability_desc_tokens (#314) ships the token TYPE -- the durable
//! static signal; the VALUE is Hydra-script-set or runtime-scaled. Reflection
//! 019ee778.
//!
//! Read-only. cargo run -p kessel-discovery --example probe_effaction_values

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kessel::gom_reader::{read_object_fields, GomValue};
use rusqlite::{Connection, OpenFlags};

const DB: &str = "/Users/seansilvius/swtor/data/spice-7.9.a-v7.sqlite";
/// evasion's sibling effect-block objects (same FQN, different guids): the
/// canonical 969B block has no value; the buff-with-duration is in a sibling.
const GAME_IDS: &[&str] = &["fa17fb067a021e89", "64b463089a096920", "f194305ff559b757"];

fn flag(n: f64) -> &'static str {
    for o in [3.0, 200.0, 2.0, 0.2] {
        if (n - o).abs() < 0.001 {
            return "  <== ORACLE";
        }
    }
    ""
}

fn dump(v: &GomValue, path: &str, depth: usize) {
    if depth > 16 {
        return;
    }
    match v {
        GomValue::Embedded(fields) => {
            for (id, val) in fields {
                dump(val, &format!("{path}.{:08x}", *id as u32), depth + 1);
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
        GomValue::F32(f) => println!("  {path} = f32 {f}{}", flag(*f as f64)),
        GomValue::I64(n) | GomValue::Enum(n) => {
            println!("  {path} = int {n}{}", flag(*n as f64))
        }
        GomValue::U64(n) => println!("  {path} = u64 {n}{}", flag(*n as f64)),
        GomValue::ClassRef(g) => println!("  {path} = ClassRef {g:016x}"),
        GomValue::Str(s) => println!("  {path} = str {s:?}"),
        _ => {}
    }
}

fn main() -> Result<()> {
    let conn = Connection::open_with_flags(DB, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    for gid in GAME_IDS {
        let b64: Option<String> = conn
            .query_row(
                "SELECT json_extract(json,'$.payload_b64') FROM objects WHERE game_id=?1",
                [gid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let Some(b64) = b64 else {
            println!("== {gid}: no payload ==");
            continue;
        };
        let payload = BASE64.decode(&b64)?;
        println!("\n== {gid} ({} bytes) ==", payload.len());
        match read_object_fields(&payload) {
            Ok(obj) => dump(&obj, "root", 0),
            Err(e) => println!("  decode err: {e}"),
        }
    }
    Ok(())
}
