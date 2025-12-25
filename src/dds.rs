//! DDS texture file handling
//!
//! Extracts DDS textures from SWTOR archives and converts to PNG.
//! Uses sha256(icon_name)[0:16] for cache-friendly filenames.
//!
//! Folder structure:
//!   icons/abilities/ab/{hash}.png
//!   icons/items/cd/{hash}.png
//!   icons/gear/ef/{hash}.png
//!   icons/misc/12/{hash}.png

use crate::hash::compute_icon_id;
use anyhow::{Context, Result};
use image_dds::ddsfile::Dds;
use image_dds::image::{codecs::png::PngEncoder, ImageEncoder, RgbaImage};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Icon type categories based on filename prefix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconType {
    Ability,    // abl_*
    Item,       // itm_*
    Gear,       // ipp.* (item preview/gear icons)
    Weapon,     // Various weapon prefixes
    Mount,      // mount_*, vehicle_*
    Companion,  // comp_*
    Achievement, // ach_*
    Misc,       // Everything else
}

impl IconType {
    /// Determine icon type from filename
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();

        if lower.starts_with("abl_") {
            IconType::Ability
        } else if lower.starts_with("itm_") || lower.starts_with("itm.") {
            IconType::Item
        } else if lower.starts_with("ipp.") || lower.starts_with("ipp_") {
            IconType::Gear
        } else if lower.starts_with("mount_") || lower.starts_with("vehicle_") || lower.starts_with("mtx_mount") {
            IconType::Mount
        } else if lower.starts_with("comp_") || lower.starts_with("companion_") {
            IconType::Companion
        } else if lower.starts_with("ach_") {
            IconType::Achievement
        } else if lower.starts_with("saber")
            || lower.starts_with("rifle")
            || lower.starts_with("blaster")
            || lower.starts_with("sniper")
            || lower.starts_with("pistol")
            || lower.starts_with("cannon")
            || lower.starts_with("electrostaff")
            || lower.starts_with("vibro")
            || lower.starts_with("techblade")
            || lower.starts_with("techstaff")
            || lower.contains("_weapon_")
        {
            IconType::Weapon
        } else {
            IconType::Misc
        }
    }

    /// Get folder name for this icon type
    pub fn folder_name(&self) -> &'static str {
        match self {
            IconType::Ability => "abilities",
            IconType::Item => "items",
            IconType::Gear => "gear",
            IconType::Weapon => "weapons",
            IconType::Mount => "mounts",
            IconType::Companion => "companions",
            IconType::Achievement => "achievements",
            IconType::Misc => "misc",
        }
    }
}

/// Extract icon name from full SWTOR path
/// "/resources/gfx/icons/abl_si_sa_cracklingblasts.dds" -> "abl_si_sa_cracklingblasts"
pub fn extract_icon_name(path: &str) -> Option<&str> {
    let filename = path.rsplit('/').next()?;
    filename.strip_suffix(".dds")
}

/// Check if a path is an icon file
pub fn is_icon_path(path: &str) -> bool {
    path.contains("/gfx/icons/") && path.ends_with(".dds")
}

/// Convert DDS data to RGBA image
pub fn dds_to_rgba(data: &[u8]) -> Result<RgbaImage> {
    let dds = Dds::read(&mut Cursor::new(data)).context("Failed to parse DDS header")?;
    let image = image_dds::image_from_dds(&dds, 0).context("Failed to decode DDS texture")?;
    Ok(image)
}

/// Convert DDS data to PNG bytes
pub fn dds_to_png(data: &[u8]) -> Result<Vec<u8>> {
    let rgba = dds_to_rgba(data)?;

    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image_dds::image::ColorType::Rgba8,
        )
        .context("Failed to encode PNG")?;

    Ok(png_bytes)
}

/// Build output path for an icon
///
/// Structure: {base_dir}/{type}/{hash_prefix}/{hash}.png
/// Example: icons/abilities/ab/abcd1234efgh5678.png
pub fn icon_output_path(base_dir: &Path, icon_name: &str) -> PathBuf {
    let icon_id = compute_icon_id(icon_name);
    let icon_type = IconType::from_name(icon_name);
    let hash_prefix = &icon_id[..2]; // First 2 chars for subfolder

    base_dir
        .join(icon_type.folder_name())
        .join(hash_prefix)
        .join(format!("{}.png", icon_id))
}

/// Save DDS as PNG with organized folder structure
///
/// Structure: {output_dir}/{type}/{hash_prefix}/{hash}.png
/// Returns the icon_id on success.
pub fn save_icon_as_png(data: &[u8], icon_path: &str, output_dir: &Path) -> Result<String> {
    let icon_name = extract_icon_name(icon_path).context("Invalid icon path - no filename")?;
    let icon_id = compute_icon_id(icon_name);

    let output_path = icon_output_path(output_dir, icon_name);

    // Skip if already exists (idempotent)
    if output_path.exists() {
        return Ok(icon_id);
    }

    // Create parent directories
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let png_bytes = dds_to_png(data)?;
    fs::write(&output_path, png_bytes)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    Ok(icon_id)
}

/// Icon extraction statistics
#[derive(Debug, Default)]
pub struct IconStats {
    pub total: usize,
    pub success: usize,
    pub errors: usize,
    pub by_type: HashMap<IconType, usize>,
}

impl IconStats {
    pub fn record_success(&mut self, icon_type: IconType) {
        self.total += 1;
        self.success += 1;
        *self.by_type.entry(icon_type).or_insert(0) += 1;
    }

    pub fn record_error(&mut self) {
        self.total += 1;
        self.errors += 1;
    }
}

/// Write icon mapping to JSON file
///
/// Creates a lookup table from icon_name -> icon_id (sha256 hash)
/// for use in ETL scripts.
pub fn write_icon_mapping(mapping: &[(String, String)], output_path: &Path) -> Result<()> {
    // Group by lowercase name for case-insensitive lookup
    let json = serde_json::to_string_pretty(
        &mapping
            .iter()
            .map(|(name, id)| {
                serde_json::json!({
                    "name": name,
                    "id": id,
                    "type": IconType::from_name(name).folder_name()
                })
            })
            .collect::<Vec<_>>(),
    )?;

    fs::write(output_path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_icon_name() {
        assert_eq!(
            extract_icon_name("/resources/gfx/icons/abl_si_sa_cracklingblasts.dds"),
            Some("abl_si_sa_cracklingblasts")
        );
        assert_eq!(
            extract_icon_name("/resources/gfx/icons/saber.mtx01_a01_v01.dds"),
            Some("saber.mtx01_a01_v01")
        );
        assert_eq!(extract_icon_name("simple.dds"), Some("simple"));
        assert_eq!(extract_icon_name("no_extension"), None);
    }

    #[test]
    fn test_is_icon_path() {
        assert!(is_icon_path("/resources/gfx/icons/abl_foo.dds"));
        assert!(!is_icon_path("/resources/gfx/textures/foo.dds"));
        assert!(!is_icon_path("/resources/gfx/icons/foo.png"));
    }

    #[test]
    fn test_icon_type_detection() {
        assert_eq!(IconType::from_name("abl_si_sa_cracklingblasts"), IconType::Ability);
        assert_eq!(IconType::from_name("itm_something"), IconType::Item);
        assert_eq!(IconType::from_name("ipp.class.war.a11.c01.s01.waist_v01"), IconType::Gear);
        assert_eq!(IconType::from_name("saber.mtx01_a01_v01"), IconType::Weapon);
        assert_eq!(IconType::from_name("rifle_high08_a02_v03"), IconType::Weapon);
        assert_eq!(IconType::from_name("mount_speeder_01"), IconType::Mount);
        assert_eq!(IconType::from_name("random_thing"), IconType::Misc);
    }

    #[test]
    fn test_icon_output_path() {
        let base = Path::new("/tmp/icons");
        let path = icon_output_path(base, "abl_si_sa_cracklingblasts");

        // Should be: /tmp/icons/abilities/{hash_prefix}/{hash}.png
        assert!(path.to_string_lossy().contains("/abilities/"));
        assert!(path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn test_icon_id_deterministic() {
        let id1 = compute_icon_id("abl_si_sa_cracklingblasts");
        let id2 = compute_icon_id("abl_si_sa_cracklingblasts");
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 16);
    }
}
