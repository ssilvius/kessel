//! Typed-value GOM reader for prototype singletons.
//!
//! SWTOR prototype objects are stored as a self-describing binary stream. The
//! grammar mirrors the reference `GomBinaryReader` / `ScriptObjectReader` from
//! GomLib: a variable-width number codec plus a one-byte type tag in front of
//! every value, so the stream can be walked without the client's class/field
//! schema (the DOM). Only the field *names* need the schema; the field *values*
//! and their delta-coded field ids are fully recoverable from the bytes.
//!
//! Number codec (`read_number`):
//! - `b < 0xC0`             -> the literal byte value
//! - `0xC0 ..= 0xC7`        -> read `b - 0xBF` following bytes, big-endian
//! - `0xC8 ..= 0xCF`        -> read `b - 0xC7` following bytes, big-endian
//! - `0xD0`                 -> the next single byte
//! - `0xD2`                 -> a lookup-list flag; skip and re-read
//!
//! `read_signed_number` shares the codec but negates the `0xC0..=0xC7` range.
//!
//! Type tags (`GomTypeId`): `01` UInt64, `02` Int64, `03` Bool, `04` Float,
//! `05` Enum, `06` String, `07` List, `08` Map, `09` EmbeddedClass,
//! `0F` ClassRef.
//!
//! Container layout (each container re-reads its element type tag inline):
//! - List: `<elem_tag> <len> <len2> { <idx> <value> }*`  (`len == len2`)
//! - Map:  `<key_tag> <val_tag> <len> <len2> { <key> <value> }*`
//!
//! A value is read by `Reader::read_value(tag)` with the cursor positioned just
//! past `tag`.

use anyhow::{bail, Result};

/// GOM type tag bytes. Only the variants observed in the itemization
/// singletons are decoded; anything else is an error so a grammar drift is
/// caught loudly rather than silently mis-parsed.
mod tag {
    pub const UINT64: u8 = 0x01;
    pub const INT64: u8 = 0x02;
    pub const BOOL: u8 = 0x03;
    pub const FLOAT: u8 = 0x04;
    pub const ENUM: u8 = 0x05;
    pub const STRING: u8 = 0x06;
    pub const LIST: u8 = 0x07;
    pub const MAP: u8 = 0x08;
    pub const EMBEDDED: u8 = 0x09;
    pub const CLASS_REF: u8 = 0x0F;
}

/// A decoded GOM value. Enums carry the zero-based member index (the engine
/// stores it one-based; the codec corrects it). Embedded objects retain each
/// field's delta-summed id so callers can match a field by id low32 rather
/// than by position.
#[derive(Debug, Clone, PartialEq)]
pub enum GomValue {
    Null,
    U64(u64),
    I64(i64),
    Bool(bool),
    F32(f32),
    Enum(i64),
    Str(String),
    List(Vec<GomValue>),
    Map(Vec<(GomValue, GomValue)>),
    Embedded(Vec<(u64, GomValue)>),
    ClassRef(u64),
}

impl GomValue {
    /// The integer payload of any numeric variant. `Float` is rounded (stat
    /// values such as `itmEquipModStats` are whole numbers stored as f32).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            GomValue::I64(v) | GomValue::Enum(v) => Some(*v),
            GomValue::U64(v) => Some(*v as i64),
            GomValue::F32(v) => Some(v.round() as i64),
            _ => None,
        }
    }

    /// Map entries, if this is a `Map`.
    pub fn as_map(&self) -> Option<&[(GomValue, GomValue)]> {
        match self {
            GomValue::Map(m) => Some(m),
            _ => None,
        }
    }

    /// List entries, if this is a `List`.
    pub fn as_list(&self) -> Option<&[GomValue]> {
        match self {
            GomValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// In an `Embedded` object, the value of the field whose id low32 matches
    /// `id_low32`. GOM fields in one object share their id high32, so the low32
    /// uniquely identifies a field within the object.
    pub fn embedded_field(&self, id_low32: u32) -> Option<&GomValue> {
        match self {
            GomValue::Embedded(fields) => fields
                .iter()
                .find(|(id, _)| (*id as u32) == id_low32)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// In an `Embedded` object, the first field whose value is itself a `Map`.
    /// Selecting by shape (rather than a field id) is deliberate: GOM field
    /// ids drift across game patches, but a modifier-package object's only
    /// map is its stat-split (`itmModPkgAttributePercentages`).
    pub fn embedded_first_map(&self) -> Option<&GomValue> {
        match self {
            GomValue::Embedded(fields) => fields
                .iter()
                .map(|(_, v)| v)
                .find(|v| matches!(v, GomValue::Map(_))),
            _ => None,
        }
    }
}

/// Forward-only cursor over a prototype payload.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// A reader positioned at `pos` within `buf`.
    pub fn new(buf: &'a [u8], pos: usize) -> Self {
        Reader { buf, pos }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let b = *self
            .buf
            .get(self.pos)
            .ok_or_else(|| anyhow::anyhow!("gom_reader: unexpected end of payload"))?;
        self.pos += 1;
        Ok(b)
    }

    fn read_be(&mut self, n: usize) -> Result<u64> {
        let mut val: u64 = 0;
        for _ in 0..n {
            val = (val << 8) | self.read_u8()? as u64;
        }
        Ok(val)
    }

    /// Variable-width unsigned number.
    pub fn read_number(&mut self) -> Result<u64> {
        let mut b = self.read_u8()?;
        if b == 0xD2 {
            // Lookup-list flag; the real prefix follows.
            b = self.read_u8()?;
        }
        match b {
            0x00..=0xBF => Ok(b as u64),
            0xC0..=0xC7 => self.read_be((b - 0xBF) as usize),
            0xC8..=0xCF => self.read_be((b - 0xC7) as usize),
            0xD0 => Ok(self.read_u8()? as u64),
            other => bail!("gom_reader: unknown number prefix 0x{other:02X}"),
        }
    }

    /// Variable-width signed number. The `0xC0..=0xC7` prefix range is the
    /// negative magnitude; `0xC8..=0xCF` is positive.
    pub fn read_signed_number(&mut self) -> Result<i64> {
        let b = self.read_u8()?;
        if b == 0xD2 {
            let len = self.read_u8()? as usize;
            // Numeric-as-string form: ASCII digits of length `len`.
            let start = self.pos;
            let end = start
                .checked_add(len)
                .filter(|e| *e <= self.buf.len())
                .ok_or_else(|| anyhow::anyhow!("gom_reader: signed string overrun"))?;
            let s = std::str::from_utf8(&self.buf[start..end])?;
            self.pos = end;
            return Ok(s.parse::<i64>()?);
        }
        match b {
            0x00..=0xBF => Ok(b as i64),
            0xC0..=0xC7 => Ok(-(self.read_be((b - 0xBF) as usize)? as i64)),
            0xC8..=0xCF => Ok(self.read_be((b - 0xC7) as usize)? as i64),
            0xD0 => Ok(0),
            other => bail!("gom_reader: unknown signed prefix 0x{other:02X}"),
        }
    }

    fn read_f32(&mut self) -> Result<f32> {
        let start = self.pos;
        let end = start
            .checked_add(4)
            .filter(|e| *e <= self.buf.len())
            .ok_or_else(|| anyhow::anyhow!("gom_reader: float overrun"))?;
        let bytes: [u8; 4] = self.buf[start..end].try_into().unwrap();
        self.pos = end;
        Ok(f32::from_le_bytes(bytes))
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_number()? as usize;
        let start = self.pos;
        let end = start
            .checked_add(len)
            .filter(|e| *e <= self.buf.len())
            .ok_or_else(|| anyhow::anyhow!("gom_reader: string overrun"))?;
        let s = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
        self.pos = end;
        Ok(s)
    }

    /// Read the next type tag and dispatch to the matching value reader. This
    /// is the inline `Load(reader, false)` of the reference: it consumes one
    /// tag byte then the value body.
    fn read_tagged(&mut self) -> Result<GomValue> {
        let tag = self.read_u8()?;
        self.read_value(tag)
    }

    /// Read a value of the given `tag` with the cursor positioned past `tag`.
    pub fn read_value(&mut self, tag: u8) -> Result<GomValue> {
        match tag {
            0x00 => Ok(GomValue::Null),
            tag::UINT64 => Ok(GomValue::U64(self.read_number()?)),
            tag::INT64 => Ok(GomValue::I64(self.read_signed_number()?)),
            tag::BOOL => Ok(GomValue::Bool(self.read_u8()? != 0)),
            tag::FLOAT => Ok(GomValue::F32(self.read_f32()?)),
            // Enum is one-based on the wire; correct to a zero-based index.
            tag::ENUM => Ok(GomValue::Enum(self.read_number()? as i64 - 1)),
            tag::STRING => Ok(GomValue::Str(self.read_string()?)),
            tag::LIST => self.read_list(),
            tag::MAP => self.read_map(),
            tag::EMBEDDED => self.read_object(),
            tag::CLASS_REF => Ok(GomValue::ClassRef(self.read_number()?)),
            other => bail!("gom_reader: unsupported type tag 0x{other:02X}"),
        }
    }

    /// `<elem_tag> <len> <len2> { <idx> <value> }*`
    fn read_list(&mut self) -> Result<GomValue> {
        let elem_tag = self.read_u8()?;
        let len = self.read_number()?;
        let len2 = self.read_number()?;
        if len != len2 {
            bail!("gom_reader: list length mismatch ({len} != {len2})");
        }
        let mut items = Vec::with_capacity(len.min(1 << 20) as usize);
        for _ in 0..len {
            let _idx = self.read_number()?;
            items.push(self.read_value(elem_tag)?);
        }
        Ok(GomValue::List(items))
    }

    /// `<key_tag> <val_tag> <len> <len2> { <key> <value> }*`
    fn read_map(&mut self) -> Result<GomValue> {
        let key_tag = self.read_u8()?;
        let val_tag = self.read_u8()?;
        let len = self.read_number()?;
        let len2 = self.read_number()?;
        if len != len2 {
            bail!("gom_reader: map length mismatch ({len} != {len2})");
        }
        let mut entries = Vec::with_capacity(len.min(1 << 20) as usize);
        for _ in 0..len {
            let key = self.read_value(key_tag)?;
            let val = self.read_value(val_tag)?;
            entries.push((key, val));
        }
        Ok(GomValue::Map(entries))
    }

    /// An embedded object: `<script_type_id> <num_fields> { <delta_id> <tag>
    /// <value> }*`. Field ids are delta-coded; the running sum is retained per
    /// field so callers can match by id low32.
    fn read_object(&mut self) -> Result<GomValue> {
        let _script_type_id = self.read_number()?;
        let num_fields = self.read_number()?;
        let mut fields = Vec::with_capacity(num_fields.min(1 << 16) as usize);
        let mut field_id: u64 = 0;
        for _ in 0..num_fields {
            field_id = field_id.wrapping_add(self.read_number()?);
            let value = self.read_tagged()?;
            fields.push((field_id, value));
        }
        Ok(GomValue::Embedded(fields))
    }
}

/// `cf 40 00 00` marks the start of a field id in a prototype payload (the
/// `0xCF` is a number prefix for an 8-byte big-endian id).
const FIELD_MARKER: [u8; 4] = [0xCF, 0x40, 0x00, 0x00];

/// Decode the first top-level field value of a prototype singleton: locate its
/// field marker, consume the field id, then read the tagged value. The
/// itemization singletons each carry their table as the first field, so this
/// yields the whole table (nested containers and embedded objects included).
pub fn read_first_field(payload: &[u8]) -> Result<GomValue> {
    let marker = payload
        .windows(FIELD_MARKER.len())
        .position(|w| w == FIELD_MARKER)
        .ok_or_else(|| anyhow::anyhow!("gom_reader: no field marker in payload"))?;
    let mut reader = Reader::new(payload, marker);
    let _field_id = reader.read_number()?;
    let tag = reader.read_u8()?;
    reader.read_value(tag)
}

/// Decode a whole GOM object payload as an `Embedded` value (its delta-coded
/// fields keyed by id). Item/NPC/etc. object payloads begin with zero padding
/// followed by the `ScriptObjectReader` stream (script type, field count, then
/// `<delta_id><tag><value>` per field); this skips the padding and walks it.
/// Field values that use an unsupported tag abort the walk, so callers get the
/// fields decoded up to that point via the returned error's context only when
/// the whole walk succeeds.
pub fn read_object_fields(payload: &[u8]) -> Result<GomValue> {
    let start = payload
        .iter()
        .position(|&b| b != 0)
        .ok_or_else(|| anyhow::anyhow!("gom_reader: payload is all zeros"))?;
    let mut reader = Reader::new(payload, start);
    reader.read_object()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_literal_and_widths() {
        // literal
        assert_eq!(Reader::new(&[0x7F], 0).read_number().unwrap(), 0x7F);
        // 0xC8 -> 1 following byte
        assert_eq!(Reader::new(&[0xC8, 0xC9], 0).read_number().unwrap(), 0xC9);
        // 0xC9 -> 2 following bytes, big-endian
        assert_eq!(
            Reader::new(&[0xC9, 0x03, 0xE8], 0).read_number().unwrap(),
            1000
        );
    }

    #[test]
    fn signed_negative_range() {
        // 0xC0 -> 1 byte, negated
        assert_eq!(
            Reader::new(&[0xC0, 0x05], 0).read_signed_number().unwrap(),
            -5
        );
        // 0xC8 -> 1 byte, positive
        assert_eq!(
            Reader::new(&[0xC8, 0x05], 0).read_signed_number().unwrap(),
            5
        );
        assert_eq!(Reader::new(&[0x00], 0).read_signed_number().unwrap(), 0);
    }

    #[test]
    fn map_of_int_to_int() {
        // Map<Int64,Int64> len 2: {0->0, 8->1}. read_value(MAP) reads the body
        // from pos: key_tag=02 val_tag=02 len=02 len2=02 then (00,00)(08,01).
        let buf = [0x02, 0x02, 0x02, 0x02, 0x00, 0x00, 0x08, 0x01];
        let mut r = Reader::new(&buf, 0);
        let v = r.read_value(tag::MAP).unwrap();
        let m = v.as_map().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].0.as_i64(), Some(0));
        assert_eq!(m[1].0.as_i64(), Some(8));
        assert_eq!(m[1].1.as_i64(), Some(1));
    }

    #[test]
    fn list_reads_and_discards_index() {
        // List<Int64> len 3: elem_tag=02 len=03 len2=03 then (idx,val)*
        let buf = [0x02, 0x03, 0x03, 0x00, 0x0A, 0x01, 0x14, 0x02, 0x1E];
        let mut r = Reader::new(&buf, 0);
        let v = r.read_value(tag::LIST).unwrap();
        let l = v.as_list().unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].as_i64(), Some(10));
        assert_eq!(l[1].as_i64(), Some(20));
        assert_eq!(l[2].as_i64(), Some(30));
    }

    #[test]
    fn enum_is_zero_based() {
        // raw 0x03 -> index 2
        assert_eq!(
            Reader::new(&[0x03], 0).read_value(tag::ENUM).unwrap(),
            GomValue::Enum(2)
        );
    }

    #[test]
    fn as_i64_rounds_float() {
        // itmEquipModStats values are whole numbers stored as f32; as_i64 must
        // round, not truncate (a truncating `as i64` would mis-state stats).
        assert_eq!(GomValue::F32(344.0).as_i64(), Some(344));
        assert_eq!(GomValue::F32(343.6).as_i64(), Some(344));
        assert_eq!(GomValue::F32(343.4).as_i64(), Some(343));
        assert_eq!(GomValue::I64(-5).as_i64(), Some(-5));
        assert_eq!(GomValue::U64(7).as_i64(), Some(7));
        assert_eq!(GomValue::Str("x".into()).as_i64(), None);
    }
}
