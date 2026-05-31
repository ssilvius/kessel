//! Full client.gom decoder.
//!
//! Walks every record, classifies it (enum/property/class), and emits:
//!   /tmp/client-gom-properties.json   (10006 property records)
//!   /tmp/client-gom-classes.json      (2220 class records, with property-list)
//!
//! Property record body format (size 33-63):
//!   0x00-0x03  u32 size
//!   0x04-0x0B  u64 id (canonical GOM property GUID -- LE bytes)
//!   0x0C-0x0F  u32 f0c (registration index / 0)
//!   0x10-0x11  u16 f10 (always 0x009A or 0x009E for properties)
//!   0x12-0x13  u16 first_str_off = 0x0020
//!   0x14-0x15  u16 (offset markers)
//!   0x16-0x17  u16 (offset markers)
//!   0x18-0x19  u16 attribute_flags
//!   0x1A-0x1B  u16 (count_or_count2)
//!   0x1C-0x1D  u16 payload_offset = 0x0020 (always)
//!   0x1E-0x1F  u16 zero
//!   0x20..     typed tail = [type_tag u8] [encoded value]
//!
//! Tail type tags (verified from 10006 records):
//!   0x01: bool (1 byte; total record sz=33)         814 records
//!   0x02: int8 (1 byte; sz=33)                     1777 records
//!   0x03: int16/short (1 byte; sz=33)              1192 records
//!   0x04: int32 (1 byte; sz=33)                     974 records
//!   0x05: u64 ref (8 bytes; sz=41) -- enum/class    510 records
//!   0x06: float32 (1 byte; sz=33)                   869 records
//!   0x07: string ref (1+1 bytes; sz=34..)           760 records
//!   0x08: array/list (variable; sz=35-63)          1507 records
//!   0x09: u64 ref (8 bytes; sz=41) -- typeref       169 records
//!   0x0e: ?? (sz=33)                                  9 records
//!   0x0f: u64 ref (8 bytes; sz=41) -- class ref     708 records
//!   0x11: ?? (sz=33)                                133 records
//!   0x12: ?? (sz=33)                                342 records
//!   0x14: ?? (sz=33)                                 63 records
//!   0x15: ?? (sz=33)                                179 records
//!
//! Class record body format (size 56-2360):
//!   0x00-0x03  u32 size
//!   0x04-0x0B  u64 class_id  (high32 = well-known GOM type id, e.g. D954FB01 for tal)
//!   0x0C-0x0F  u32 f0c       (0x40000040)
//!   0x10-0x11  u16 f10       (0x00A2 or 0x00A6)
//!   0x12-0x13  u16 first_str_off = 0x0034
//!   0x14-0x17  u16 (offset markers)
//!   0x18-0x1F  u64 parent_class_a (D000XXXXXXXXXXXX)
//!   0x20-0x27  u64 parent_class_b (D000XXXXXXXXXXXX)
//!   0x28-0x29  u16 (flag1, often 0x0001)
//!   0x2A-0x2B  u16 prop_count_b
//!   0x2C-0x2D  u16 prop_list_start (= 0x38)
//!   0x2E-0x2F  u16 prop_count
//!   0x30-0x37  u64 padding (0x40 high byte)
//!   0x38..end  [u64 property_ref] * prop_count -- each is a CF40-style template GUID

use serde::Serialize;
use std::fs;

#[derive(Serialize)]
struct PropertyRecord {
    index: usize,
    offset_hex: String,
    size: usize,
    id_hex: String,
    f0c_hex: String,
    f10_hex: String,
    flags_a_hex: String,
    flags_b_hex: String,
    type_tag: String,
    type_tag_decimal: u8,
    typed_value_hex: String,
    /// 8-byte ref value as u64 if type is a ref-type (0x05, 0x09, 0x0F).
    ref_value_hex: Option<String>,
}

#[derive(Serialize)]
struct ClassRecord {
    index: usize,
    offset_hex: String,
    size: usize,
    class_id_hex: String,
    class_type_hi32: String,
    parent_a_hex: String,
    parent_b_hex: String,
    prop_count: u16,
    property_refs: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let data = fs::read("/tmp/client.gom.bin")?;
    let mut pos: usize = 8;
    let mut idx = 0;

    let mut props: Vec<PropertyRecord> = Vec::new();
    let mut classes: Vec<ClassRecord> = Vec::new();

    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into()?) as usize;
        if size == 0 {
            pos += 1;
            continue;
        }
        if size < 16 || pos + size > data.len() {
            break;
        }
        let body = &data[pos..pos + size];

        if body.len() >= 0x21 {
            let fso = u16::from_le_bytes(body[0x12..0x14].try_into()?);
            let id = u64::from_le_bytes(body[0x04..0x0C].try_into()?);
            let f0c = u32::from_le_bytes(body[0x0C..0x10].try_into()?);
            let f10 = u16::from_le_bytes(body[0x10..0x12].try_into()?);

            if fso == 0x20 {
                // Property record
                let flags_a = u16::from_le_bytes(body[0x18..0x1A].try_into()?);
                let flags_b = u16::from_le_bytes(body[0x1A..0x1C].try_into()?);
                let type_tag = body[0x20];
                let tail = &body[0x20..];
                let typed_hex: String = tail.iter().map(|b| format!("{:02X}", b)).collect();
                let ref_value = match type_tag {
                    0x05 | 0x09 | 0x0F if tail.len() >= 9 => {
                        let v = u64::from_le_bytes(tail[1..9].try_into()?);
                        Some(format!("{:016X}", v))
                    }
                    _ => None,
                };
                props.push(PropertyRecord {
                    index: idx,
                    offset_hex: format!("{:#08x}", pos),
                    size,
                    id_hex: format!("{:016X}", id),
                    f0c_hex: format!("{:08X}", f0c),
                    f10_hex: format!("{:04X}", f10),
                    flags_a_hex: format!("{:04X}", flags_a),
                    flags_b_hex: format!("{:04X}", flags_b),
                    type_tag: format!("{:02X}", type_tag),
                    type_tag_decimal: type_tag,
                    typed_value_hex: typed_hex,
                    ref_value_hex: ref_value,
                });
            } else if fso == 0x34 && body.len() >= 0x38 {
                // Class record
                let parent_a = u64::from_le_bytes(body[0x18..0x20].try_into()?);
                let parent_b = u64::from_le_bytes(body[0x20..0x28].try_into()?);
                let prop_count = u16::from_le_bytes(body[0x2E..0x30].try_into()?);
                let mut refs = Vec::new();
                let mut i = 0x38;
                while i + 8 <= size {
                    let v = u64::from_le_bytes(body[i..i + 8].try_into()?);
                    refs.push(format!("{:016X}", v));
                    i += 8;
                }
                classes.push(ClassRecord {
                    index: idx,
                    offset_hex: format!("{:#08x}", pos),
                    size,
                    class_id_hex: format!("{:016X}", id),
                    class_type_hi32: format!("{:08X}", (id >> 32) as u32),
                    parent_a_hex: format!("{:016X}", parent_a),
                    parent_b_hex: format!("{:016X}", parent_b),
                    prop_count,
                    property_refs: refs,
                });
            }
        }

        let next = pos + size;
        let pad = (8 - (next % 8)) % 8;
        pos = next + pad;
        idx += 1;
    }

    println!(
        "Decoded {} property records, {} class records",
        props.len(),
        classes.len()
    );

    let pj = serde_json::to_string_pretty(&props)?;
    fs::write("/tmp/client-gom-properties.json", &pj)?;
    println!("Wrote /tmp/client-gom-properties.json ({} bytes)", pj.len());

    let cj = serde_json::to_string_pretty(&classes)?;
    fs::write("/tmp/client-gom-classes.json", &cj)?;
    println!("Wrote /tmp/client-gom-classes.json ({} bytes)", cj.len());

    Ok(())
}
