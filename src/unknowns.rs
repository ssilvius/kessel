//! Unknown pattern tracker for iterative format discovery
//!
//! Captures anything we don't fully understand during extraction:
//! - Unknown FQN prefixes
//! - Failed decompression
//! - Missing expected fields
//! - Unrecognized byte patterns
//!
//! Output: JSONL file for analysis and ML training

use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

/// Types of unknown patterns we track
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum Unknown {
    /// FQN prefix not in our known list
    #[serde(rename = "unknown_prefix")]
    UnknownPrefix {
        prefix: String,
        fqn: String,
        sample_count: u32,
    },

    /// ZSTD decompression failed
    #[serde(rename = "decompress_failed")]
    DecompressFailed {
        fqn: String,
        header_hex: String,
        error: String,
    },

    /// Icon referenced but not found in any object
    #[serde(rename = "unmapped_icon")]
    UnmappedIcon {
        icon_name: String,
        source_file: String,
    },

    /// Object missing string_id when we expected one
    #[serde(rename = "missing_string_id")]
    MissingStringId {
        fqn: String,
        kind: String,
        has_icon: bool,
    },

    /// Unrecognized file in archive (not PBUK, DBLB, STB, DDS)
    #[serde(rename = "unknown_file_type")]
    UnknownFileType {
        path: String,
        magic_hex: String,
        size: usize,
    },

    /// MYP entry with hash but no known filename
    #[serde(rename = "orphan_hash")]
    OrphanHash {
        hash1: u32,
        hash2: u32,
        size: usize,
        compressed: bool,
    },

    /// Byte pattern we haven't decoded yet
    #[serde(rename = "unknown_pattern")]
    UnknownPattern {
        context: String,
        offset: usize,
        pattern_hex: String,
        surrounding_hex: String,
    },
}

/// Buffered JSONL writer for unknowns
pub struct UnknownsWriter {
    writer: Mutex<Option<BufWriter<File>>>,
    prefix_counts: Mutex<std::collections::HashMap<String, u32>>,
}

impl UnknownsWriter {
    /// Create a new unknowns writer
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        Ok(Self {
            writer: Mutex::new(Some(BufWriter::new(file))),
            prefix_counts: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Create a no-op writer (for when unknowns tracking is disabled)
    pub fn disabled() -> Self {
        Self {
            writer: Mutex::new(None),
            prefix_counts: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Record an unknown pattern
    pub fn record(&self, unknown: Unknown) {
        let mut writer_guard = self.writer.lock().unwrap();
        if let Some(ref mut writer) = *writer_guard {
            if let Ok(json) = serde_json::to_string(&unknown) {
                let _ = writeln!(writer, "{}", json);
            }
        }
    }

    /// Flush and finalize - writes summary of prefix counts
    pub fn finalize(&self) -> std::io::Result<()> {
        // Write final prefix counts
        let counts = self.prefix_counts.lock().unwrap();
        for (prefix, count) in counts.iter() {
            if *count > 1 {
                self.record(Unknown::UnknownPrefix {
                    prefix: prefix.clone(),
                    fqn: format!("(final count: {} occurrences)", count),
                    sample_count: *count,
                });
            }
        }

        // Flush writer
        let mut writer_guard = self.writer.lock().unwrap();
        if let Some(ref mut writer) = *writer_guard {
            writer.flush()?;
        }

        Ok(())
    }
}
