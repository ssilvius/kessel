//! MYP Archive Reader
//!
//! .tor files use the MYP format with the following structure:
//!
//! Header (40 bytes):
//!   - bytes 0-2: "MYP" identifier
//!   - bytes 3-10: offset to first file table (u64)
//!   - bytes 11-14: max files in first table (u32)
//!   - bytes 15-18: total file count (u32)
//!   - bytes 19-22: number of file tables (u32)
//!
//! File Table Header (12 bytes):
//!   - bytes 0-3: max files in this table (u32)
//!   - bytes 4-11: offset to next table or 0 (u64)
//!
//! File Entry (34 bytes):
//!   - bytes 0-7: position in archive (u64)
//!   - bytes 8-11: header size (u32)
//!   - bytes 12-15: compressed size (u32)
//!   - bytes 16-19: uncompressed size (u32)
//!   - bytes 20-27: filename hash (u64)
//!   - bytes 28-31: crc32 (u32)
//!   - bytes 32-33: compression flag (u16) - 0=none, 1=zlib

use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const MYP_MAGIC: [u8; 3] = [b'M', b'Y', b'P'];
const HEADER_SIZE: usize = 40;
const FILE_TABLE_HEADER_SIZE: usize = 12;
const FILE_ENTRY_SIZE: usize = 34;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub position: u64,
    pub header_size: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub filename_hash: u64,
    pub crc32: u32,
    pub compression: u16,
}

pub struct Archive {
    reader: BufReader<File>,
    entries: Vec<FileEntry>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).context("Failed to open .tor file")?;
        let mut reader = BufReader::new(file);

        // Read and verify header
        let mut header = [0u8; HEADER_SIZE];
        reader.read_exact(&mut header)?;

        if header[0..3] != MYP_MAGIC {
            bail!("Invalid MYP file: bad magic number");
        }

        // Parse header fields
        let first_table_offset = u64::from_le_bytes(header[3..11].try_into()?);
        let _first_max_files = u32::from_le_bytes(header[11..15].try_into()?);
        let total_files = u32::from_le_bytes(header[15..19].try_into()?);
        let _num_tables = u32::from_le_bytes(header[19..23].try_into()?);

        tracing::debug!("MYP header: {} files, first table at {}", total_files, first_table_offset);

        // Read all file tables
        let mut entries = Vec::with_capacity(total_files as usize);
        let mut table_offset = first_table_offset;

        while table_offset != 0 {
            reader.seek(SeekFrom::Start(table_offset))?;

            // Read table header
            let mut table_header = [0u8; FILE_TABLE_HEADER_SIZE];
            reader.read_exact(&mut table_header)?;

            let max_files = u32::from_le_bytes(table_header[0..4].try_into()?);
            let next_table = u64::from_le_bytes(table_header[4..12].try_into()?);

            // Read file entries
            for _ in 0..max_files {
                let mut entry_data = [0u8; FILE_ENTRY_SIZE];
                reader.read_exact(&mut entry_data)?;

                let position = u64::from_le_bytes(entry_data[0..8].try_into()?);

                // Skip empty entries
                if position == 0 {
                    continue;
                }

                let entry = FileEntry {
                    position,
                    header_size: u32::from_le_bytes(entry_data[8..12].try_into()?),
                    compressed_size: u32::from_le_bytes(entry_data[12..16].try_into()?),
                    uncompressed_size: u32::from_le_bytes(entry_data[16..20].try_into()?),
                    filename_hash: u64::from_le_bytes(entry_data[20..28].try_into()?),
                    crc32: u32::from_le_bytes(entry_data[28..32].try_into()?),
                    compression: u16::from_le_bytes(entry_data[32..34].try_into()?),
                };

                entries.push(entry);
            }

            table_offset = next_table;
        }

        tracing::debug!("Loaded {} file entries", entries.len());

        Ok(Self { reader, entries })
    }

    pub fn entries(&self) -> Result<impl Iterator<Item = &FileEntry>> {
        Ok(self.entries.iter())
    }

    pub fn read_entry(&mut self, entry: &FileEntry) -> Result<Vec<u8>> {
        // Seek to file position (skip header)
        self.reader.seek(SeekFrom::Start(entry.position + entry.header_size as u64))?;

        // Read compressed data
        let mut compressed = vec![0u8; entry.compressed_size as usize];
        self.reader.read_exact(&mut compressed)?;

        // Decompress if needed
        if entry.compression == 1 {
            let mut decoder = ZlibDecoder::new(&compressed[..]);
            let mut decompressed = Vec::with_capacity(entry.uncompressed_size as usize);
            decoder.read_to_end(&mut decompressed)?;
            Ok(decompressed)
        } else {
            Ok(compressed)
        }
    }
}
