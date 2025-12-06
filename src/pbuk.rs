//! PBUK/DBLB Parser
//!
//! GOM (Game Object Model) files use a nested container format:
//!
//! PBUK Container (outer):
//!   - bytes 0-3: "PBUK" identifier
//!   - bytes 4-5: chunk count (u16)
//!   - bytes 6-7: unknown (u16)
//!   - bytes 8-11: initial chunk size (u32)
//!   - chunks: each prefixed with 4-byte size
//!
//! DBLB Format (inner, nested in PBUK chunks):
//!   - bytes 0-3: "DBLB" identifier
//!   - bytes 4-7: unknown (u32)
//!   - objects follow with 42-byte headers:
//!     - bytes 0-3: object size (u32)
//!     - bytes 4-5: data type (u16)
//!     - bytes 6-7: data offset (u16)
//!     - bytes 42-45: type marker (04 00 01 0X)
//!   - type 15 = zlib compressed content
//!   - objects have label string + compressed payload

use anyhow::{bail, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

const PBUK_MAGIC: [u8; 4] = [b'P', b'B', b'U', b'K'];
const DBLB_MAGIC: [u8; 4] = [b'D', b'B', b'L', b'B'];

/// Check if data starts with PBUK magic
pub fn is_pbuk(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == PBUK_MAGIC
}

/// Check if data starts with DBLB magic
pub fn is_dblb(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == DBLB_MAGIC
}

#[derive(Debug)]
pub struct DblbObject {
    pub label: String,
    pub data_type: u16,
    pub compressed: bool,
    pub data: Vec<u8>,
}

impl DblbObject {
    /// Decompress the object data if compressed, returning XML content
    pub fn decompress(&self) -> Result<Option<Vec<u8>>> {
        if !self.compressed || self.data.is_empty() {
            return Ok(if self.data.is_empty() { None } else { Some(self.data.clone()) });
        }

        let mut decoder = ZlibDecoder::new(&self.data[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        Ok(Some(decompressed))
    }
}

/// Parse a PBUK container, extracting all DBLB chunks
pub fn parse(data: &[u8]) -> Result<Vec<DblbObject>> {
    if !is_pbuk(data) {
        bail!("Not a PBUK file");
    }

    let chunk_count = u16::from_le_bytes([data[4], data[5]]) as usize;
    let initial_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;

    tracing::debug!("PBUK: {} chunks, initial size {}", chunk_count, initial_size);

    let mut objects = Vec::new();
    let mut offset = 12; // After PBUK header

    // Read first chunk
    if offset + initial_size <= data.len() {
        let chunk_data = &data[offset..offset + initial_size];
        if is_dblb(chunk_data) {
            objects.extend(parse_dblb(chunk_data)?);
        }
        offset += initial_size;
    }

    // Read remaining chunks (each prefixed with 4-byte size)
    for _ in 1..chunk_count {
        if offset + 4 > data.len() {
            break;
        }

        let chunk_size = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + chunk_size > data.len() {
            break;
        }

        let chunk_data = &data[offset..offset + chunk_size];
        if is_dblb(chunk_data) {
            objects.extend(parse_dblb(chunk_data)?);
        }
        offset += chunk_size;
    }

    Ok(objects)
}

/// Parse a DBLB block, extracting game objects
fn parse_dblb(data: &[u8]) -> Result<Vec<DblbObject>> {
    if !is_dblb(data) {
        bail!("Not a DBLB block");
    }

    let mut objects = Vec::new();
    let mut offset = 8; // After DBLB header + unknown bytes

    while offset + 42 < data.len() {
        // Read object header (42 bytes)
        let obj_size = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;

        if obj_size == 0 {
            break;
        }

        let data_type = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
        let data_offset = u16::from_le_bytes([data[offset + 6], data[offset + 7]]) as usize;

        // Check type marker at offset 42-45 for compression indicator
        let type_marker = if offset + 45 < data.len() {
            data[offset + 45]
        } else {
            0
        };
        let compressed = type_marker == 15;

        // Extract label (null-terminated string after header)
        let label_start = offset + 42;
        let label_end = data[label_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| label_start + p)
            .unwrap_or(label_start);
        let label = String::from_utf8_lossy(&data[label_start..label_end]).to_string();

        // Extract object data
        let obj_data_start = offset + data_offset;
        let obj_data_end = offset + obj_size;

        if obj_data_end <= data.len() && obj_data_start < obj_data_end {
            let obj_data = data[obj_data_start..obj_data_end].to_vec();

            objects.push(DblbObject {
                label,
                data_type,
                compressed,
                data: obj_data,
            });
        }

        offset += obj_size;
    }

    tracing::debug!("DBLB: parsed {} objects", objects.len());
    Ok(objects)
}
