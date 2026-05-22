//! Decrypt SCPT compiled-script files from /resources/systemgenerated/compilednative/.
//!
//! ## SCPT Format (reverse-engineered)
//!
//! All files in `swtor_main_global_1.tor` under `/resources/systemgenerated/compilednative/`
//! share this format:
//!
//! ### Header (37 bytes, plaintext)
//!
//! Offset  Size  Field           Description
//! ------  ----  -----           -----------
//! 0x00    4     magic           "SCPT" (0x53 0x43 0x50 0x54)
//! 0x04    4     version         0x06000500 (version 6.5)
//! 0x08    8     section_count?  Constant 0x0000000000000001
//! 0x10    8     guid_le         Per-file u64 LE = the numeric filename
//! 0x18    8     constant        0x0000000000000201
//! 0x20    1     pad             0x00
//! 0x21    3     body_size_le    u24 LE: bytes following the header
//! 0x24    1     pad             0x00
//!
//! ### Body (body_size bytes, encrypted)
//!
//! XOR stream cipher with the deterministic keystream:
//!
//!     plaintext[i] = cipher[i] XOR ((0x43 + 11*i) mod 256)
//!
//! The cipher key is **constant across all 1,196 files** -- no per-file derivation,
//! no GUID dependency. Confirmed via 149 stub files whose decrypted plaintext is
//! identical (30 bytes of mostly-zero data with structural tag bytes).
//!
//! ### Plaintext layout (after decryption)
//!
//! The decrypted body is a SWTOR-internal compiled-script container holding
//! native x86-64 machine code, not Pawn bytecode. 1046/1047 non-stub files
//! contain identifiable x86-64 function prologues (`55 48 89 E5`, `41 57 41 56`,
//! etc). The body opens with a metadata section that uses tag bytes:
//!
//!   0xCB = string-reference / external symbol id (5-byte record: CB + 4 bytes)
//!   0xC9 = native-code blob marker
//!   0xD1 = literal-int open, 0xD3 = literal-int close
//!
//! followed by the raw x86-64 function code. This is JIT-style precompiled
//! UI/scripting code, NOT gameplay-formula data.
//!
//! ## Important: SCPT files are UI/client logic, not gameplay math
//!
//! Identifiable script purposes (extracted from embedded string tables):
//! - `displayDialog`, `scrollPagePanel`, `OnSliderFieldValueChange`
//! - `cmdValidateIsPositiveNumber`, `HE_getWindow` (HTML-Engine UI widgets)
//! - `Play_spvp_targetlock_beep`, `onSetMissileLockIndicatorState`
//! - `displayGalacticStarfighterUpsellWindow`
//!
//! No GSF damage formulas, no per-component damage constants, no combat
//! arithmetic was found in any of the 1,196 SCPT files. The "damage" strings
//! that DO appear are UI event hooks (`onChangedPlayerDamageStat`) and
//! animation labels (`damage_01`..`damage_05`).
//!
//! Conclusion: SCPT is the wrong format for GSF damage extraction. Combat
//! formulas must live elsewhere -- likely in client EXE/DLL code that consumes
//! these scripts but performs the math itself.

use anyhow::Result;
use kessel::myp::Archive;
use std::path::PathBuf;

const SCPT_MAGIC: [u8; 4] = *b"SCPT";
const SCPT_HEADER_SIZE: usize = 37;
const CIPHER_START: u8 = 0x43;
const CIPHER_STEP: u8 = 11;

/// Decrypt an SCPT file body in-place.
///
/// `body` is the bytes AFTER the 37-byte header. Returns the plaintext
/// (same length as input).
pub fn decrypt_body(body: &[u8]) -> Vec<u8> {
    body.iter()
        .enumerate()
        .map(|(i, &b)| {
            let key = CIPHER_START.wrapping_add(CIPHER_STEP.wrapping_mul(i as u8));
            b ^ key
        })
        .collect()
}

#[derive(Debug)]
pub struct ScptHeader {
    pub version: u32,
    pub section_count: u64,
    pub guid: u64,
    pub constant_18: u64,
    pub body_size: u32,
}

impl ScptHeader {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < SCPT_HEADER_SIZE {
            anyhow::bail!("SCPT data too small: {} < {}", data.len(), SCPT_HEADER_SIZE);
        }
        if data[0..4] != SCPT_MAGIC {
            anyhow::bail!("bad SCPT magic: {:?}", &data[0..4]);
        }
        // body_size is a u24 LE at offset 0x21
        let body_size = u32::from_le_bytes([data[0x21], data[0x22], data[0x23], 0]);
        Ok(Self {
            version: u32::from_le_bytes(data[4..8].try_into()?),
            section_count: u64::from_le_bytes(data[8..16].try_into()?),
            guid: u64::from_le_bytes(data[16..24].try_into()?),
            constant_18: u64::from_le_bytes(data[24..32].try_into()?),
            body_size,
        })
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut output_dir: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-o" => {
                output_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => i += 1,
        }
    }
    let output_dir = output_dir.unwrap_or_else(|| PathBuf::from("/tmp/scpt-decrypted"));
    std::fs::create_dir_all(&output_dir)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut total = 0usize;
    for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if entry.compressed_size < 40 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < SCPT_HEADER_SIZE || data[0..4] != SCPT_MAGIC {
                continue;
            }
            let header = ScptHeader::parse(&data)?;
            let body = &data[SCPT_HEADER_SIZE..];
            if body.len() != header.body_size as usize {
                eprintln!(
                    "warn: body size mismatch for {}: hdr={} actual={}",
                    header.guid,
                    header.body_size,
                    body.len()
                );
            }
            let plaintext = decrypt_body(body);
            let out_path = output_dir.join(format!("{}.scpt.dec", header.guid));
            std::fs::write(&out_path, &plaintext)?;
            total += 1;
        }
    }
    println!(
        "decrypted {} SCPT files into {}",
        total,
        output_dir.display()
    );
    Ok(())
}
