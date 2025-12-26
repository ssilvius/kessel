//! DDS to WebP icon exporter
//!
//! Converts DirectDraw Surface (DDS) textures from SWTOR game files to WebP.
//!
//! SWTOR uses DDS textures with various compression formats:
//! - DXT1 (BC1): RGB, 1-bit alpha
//! - DXT5 (BC3): RGBA, interpolated alpha
//! - Uncompressed: RGBA8888
//!
//! WebP advantages over PNG:
//! - Much smaller file sizes (typically 25-35% smaller)
//! - Supports transparency (lossless)
//! - Well-supported in all modern browsers
//!
//! Icon uniqueness strategy:
//! - Icon ID: sha256(icon_name)[0:16] for filename - matches computeIconId() in frontend
//! - Content hash: sha256(pixel_data)[0:16] for detecting duplicate images

use anyhow::{bail, Context, Result};
use image_dds::ddsfile::Dds;
use image_dds::image::RgbaImage;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::Path;

/// DDS file magic number
const DDS_MAGIC: [u8; 4] = [b'D', b'D', b'S', b' '];

/// Check if data starts with DDS magic
pub fn is_dds(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == DDS_MAGIC
}

/// Compute content-based hash from raw pixel data
/// Returns 16-character hex string: sha256(rgba_pixels)[0:16]
pub fn compute_content_hash(pixels: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pixels);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Result of converting a DDS texture
#[derive(Debug)]
pub struct ConvertedIcon {
    /// Icon ID: sha256(icon_name)[0:16] - matches computeIconId() in frontend
    pub icon_id: String,
    /// Content-based hash (from pixel data) for deduplication
    pub content_hash: String,
    /// Icon name/path from game data
    pub icon_name: String,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// WebP data (lossless)
    pub webp_data: Vec<u8>,
}

impl ConvertedIcon {
    /// Get the deterministic filename for this icon
    /// Uses icon_id (sha256(icon_name)[0:16]) - matches computeIconId() in frontend
    pub fn filename(&self) -> String {
        format!("{}.webp", self.icon_id)
    }

    /// Check if this icon has the same content as another
    pub fn is_duplicate_of(&self, other: &ConvertedIcon) -> bool {
        self.content_hash == other.content_hash
    }
}

/// Convert DDS texture bytes to WebP
///
/// # Arguments
/// * `data` - Raw DDS file bytes
/// * `icon_name` - Icon path/name (e.g., "/resources/gfx/icons/abl_sw_ma_gore.dds")
///
/// # Returns
/// Converted icon with icon_id (sha256(name)[0:16]) and content hash
pub fn convert_to_webp(data: &[u8], icon_name: &str) -> Result<ConvertedIcon> {
    if !is_dds(data) {
        bail!("Not a DDS file: missing magic number");
    }

    // Parse DDS file using ddsfile crate
    let mut cursor = Cursor::new(data);
    let dds = Dds::read(&mut cursor).context("Failed to parse DDS header")?;

    // Decode to RGBA image using image_dds
    // mipmap level 0 = base texture
    let rgba: RgbaImage = image_dds::image_from_dds(&dds, 0)
        .context("Failed to decode DDS texture")?;

    let (width, height) = rgba.dimensions();

    // Compute content hash from raw pixels
    let pixels = rgba.as_raw();
    let content_hash = compute_content_hash(pixels);

    // Compute icon_id using icon name - matches computeIconId() in frontend
    let icon_id = crate::hash::compute_icon_id(icon_name);

    // Encode to WebP (lossless for transparency support)
    let mut webp_data = Vec::new();
    {
        use image_dds::image::ImageEncoder;
        use image_dds::image::codecs::webp::WebPEncoder;
        use image_dds::image::ColorType;
        // Use lossless encoding to preserve quality and transparency
        let encoder = WebPEncoder::new_lossless(&mut webp_data);
        encoder
            .write_image(pixels, width, height, ColorType::Rgba8)
            .context("Failed to encode WebP")?;
    }

    Ok(ConvertedIcon {
        icon_id,
        content_hash,
        icon_name: icon_name.to_string(),
        width,
        height,
        webp_data,
    })
}

/// Convert DDS file from disk to WebP
pub fn convert_file(input: &Path, icon_name: &str) -> Result<ConvertedIcon> {
    let file = File::open(input).context("Failed to open DDS file")?;
    let mut reader = BufReader::new(file);
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;

    convert_to_webp(&data, icon_name)
}

/// Save converted icon to output directory
pub fn save_icon(icon: &ConvertedIcon, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    let output_path = output_dir.join(icon.filename());
    let mut file = File::create(&output_path)?;
    file.write_all(&icon.webp_data)?;

    tracing::debug!(
        "Saved icon: {} -> {} ({}x{}, content: {})",
        icon.icon_name,
        output_path.display(),
        icon.width,
        icon.height,
        icon.content_hash
    );

    Ok(())
}

/// Batch convert multiple DDS textures
/// Returns (converted, duplicates, errors)
pub fn batch_convert<'a>(
    items: impl Iterator<Item = (&'a [u8], &'a str)>,
) -> (Vec<ConvertedIcon>, Vec<(String, String)>, Vec<(String, String)>) {
    let mut converted = Vec::new();
    let mut duplicates = Vec::new();
    let mut errors = Vec::new();
    let mut seen_content: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (data, icon_name) in items {
        match convert_to_webp(data, icon_name) {
            Ok(icon) => {
                // Check for duplicate content
                if let Some(original) = seen_content.get(&icon.content_hash) {
                    duplicates.push((icon_name.to_string(), original.clone()));
                } else {
                    seen_content.insert(icon.content_hash.clone(), icon_name.to_string());
                    converted.push(icon);
                }
            }
            Err(e) => {
                errors.push((icon_name.to_string(), e.to_string()));
            }
        }
    }

    (converted, duplicates, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal valid DDS file header (DXT1 compressed, 4x4)
    fn make_test_dds() -> Vec<u8> {
        let mut data = Vec::new();

        // DDS magic
        data.extend_from_slice(&DDS_MAGIC);

        // DDS_HEADER (124 bytes)
        // dwSize
        data.extend_from_slice(&124u32.to_le_bytes());
        // dwFlags (DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_LINEARSIZE)
        data.extend_from_slice(&0x000A1007u32.to_le_bytes());
        // dwHeight
        data.extend_from_slice(&4u32.to_le_bytes());
        // dwWidth
        data.extend_from_slice(&4u32.to_le_bytes());
        // dwPitchOrLinearSize (8 bytes for 4x4 DXT1)
        data.extend_from_slice(&8u32.to_le_bytes());
        // dwDepth
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwMipMapCount
        data.extend_from_slice(&1u32.to_le_bytes());
        // dwReserved1[11]
        data.extend_from_slice(&[0u8; 44]);

        // DDS_PIXELFORMAT (32 bytes)
        // dwSize
        data.extend_from_slice(&32u32.to_le_bytes());
        // dwFlags (DDPF_FOURCC)
        data.extend_from_slice(&0x00000004u32.to_le_bytes());
        // dwFourCC ("DXT1")
        data.extend_from_slice(b"DXT1");
        // dwRGBBitCount
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwRBitMask
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwGBitMask
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwBBitMask
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwABitMask
        data.extend_from_slice(&0u32.to_le_bytes());

        // dwCaps
        data.extend_from_slice(&0x00001000u32.to_le_bytes());
        // dwCaps2
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwCaps3
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwCaps4
        data.extend_from_slice(&0u32.to_le_bytes());
        // dwReserved2
        data.extend_from_slice(&0u32.to_le_bytes());

        // DXT1 data for 4x4 block (8 bytes)
        // Color endpoints and indices for a simple pattern
        data.extend_from_slice(&[0xFF, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        data
    }

    #[test]
    fn test_is_dds() {
        assert!(is_dds(&DDS_MAGIC));
        assert!(is_dds(b"DDS test data"));
        assert!(!is_dds(b"PNG"));
        assert!(!is_dds(&[]));
    }

    #[test]
    fn test_content_hash_deterministic() {
        let pixels = [0u8, 255, 128, 64, 32, 16, 8, 4];
        let hash1 = compute_content_hash(&pixels);
        let hash2 = compute_content_hash(&pixels);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_content_hash_different_for_different_content() {
        let pixels1 = [0u8; 16];
        let pixels2 = [255u8; 16];
        let hash1 = compute_content_hash(&pixels1);
        let hash2 = compute_content_hash(&pixels2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_convert_to_webp() {
        let dds_data = make_test_dds();
        let result = convert_to_webp(&dds_data, "/resources/gfx/icons/abl_test.dds");

        assert!(result.is_ok(), "Failed to convert: {:?}", result.err());

        let icon = result.unwrap();
        assert_eq!(icon.width, 4);
        assert_eq!(icon.height, 4);
        assert_eq!(icon.icon_name, "/resources/gfx/icons/abl_test.dds");
        assert_eq!(icon.icon_id.len(), 16);
        assert_eq!(icon.content_hash.len(), 16);
        assert!(!icon.webp_data.is_empty());

        // Verify WebP magic: "RIFF" at offset 0, "WEBP" at offset 8
        assert_eq!(&icon.webp_data[0..4], b"RIFF");
        assert_eq!(&icon.webp_data[8..12], b"WEBP");
    }

    #[test]
    fn test_convert_invalid_data() {
        let result = convert_to_webp(b"not a dds file", "/test/icon.dds");
        assert!(result.is_err());
    }

    #[test]
    fn test_filename_format() {
        let dds_data = make_test_dds();
        let icon = convert_to_webp(&dds_data, "/resources/gfx/icons/abl_test.dds").unwrap();

        let filename = icon.filename();
        assert!(filename.ends_with(".webp"));
        assert_eq!(filename.len(), 21); // 16 hex chars + ".webp"
    }

    #[test]
    fn test_duplicate_detection() {
        let dds_data = make_test_dds();

        let icon1 = convert_to_webp(&dds_data, "/icons/one.dds").unwrap();
        let icon2 = convert_to_webp(&dds_data, "/icons/two.dds").unwrap();

        // Same content, different names
        assert!(icon1.is_duplicate_of(&icon2));
        assert_ne!(icon1.icon_id, icon2.icon_id);
        assert_eq!(icon1.content_hash, icon2.content_hash);
    }

    #[test]
    fn test_batch_convert() {
        let dds_data = make_test_dds();

        let items: Vec<(&[u8], &str)> = vec![
            (&dds_data, "/icons/one.dds"),
            (&dds_data, "/icons/two.dds"), // duplicate content
            (b"invalid", "/icons/bad.dds"), // error
        ];

        let (converted, duplicates, errors) = batch_convert(items.into_iter());

        assert_eq!(converted.len(), 1);
        assert_eq!(duplicates.len(), 1);
        assert_eq!(errors.len(), 1);

        assert_eq!(duplicates[0].0, "/icons/two.dds");
        assert_eq!(duplicates[0].1, "/icons/one.dds");
        assert_eq!(errors[0].0, "/icons/bad.dds");
    }
}