//! Crafting schematic recipe and detail extraction.

use super::*;

impl Database {
    /// Populate `schematics` and `schematic_materials` from `itm.schem.*` +
    /// `schem.*` payloads.
    ///
    /// Each `itm.schem.*` object's payload carries a CF GUID ref to a
    /// companion `schem.*` object (different GOM kind, ~14k instances). The
    /// schem.* payload encodes the recipe: a list of CF GUID refs each
    /// followed by a quantity byte. Resolved FQNs are split by prefix:
    /// `itm.mat.*` rows go to `schematic_materials`, anything else is treated
    /// as the output and stored in `schematics.output_fqn`.
    ///
    /// The quantity byte sits immediately after each 9-byte CF marker
    /// (`CF E0 NN NN NN NN NN NN NN`). Material values run 1-99 in observed
    /// payloads (low-bit-set non-CF bytes); the parser clamps to 0..99 to
    /// reject obviously-non-quantity bytes.
    pub fn populate_schematic_recipes(&self) -> Result<u64> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use std::collections::HashMap;

        let conn = self.conn.lock().unwrap();

        // Build GUID -> FQN map for all objects (only need one lookup table).
        let mut guid_to_fqn: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT guid, fqn FROM objects")?;
            for row in stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
            {
                guid_to_fqn.insert(row.0.to_uppercase(), row.1);
            }
        }

        // Map itm.schem.<X> -> schem.<X> via the strip-prefix convention,
        // resolved by FQN match (cheap and reliable; the CF ref out of the
        // itm.schem.* payload would also work but adds a dump pass).
        // Build schem.* fqn -> payload_b64 map (single scan, indexed lookup).
        let schem_payloads: HashMap<String, String> = {
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'schem' AND is_canonical = 1",
            )?;
            let collected: HashMap<String, String> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        // Pair each itm.schem.* with its schem.* companion via the strip-prefix
        // convention. In-memory map lookup avoids the quadratic SQL JOIN that
        // would otherwise run REPLACE() against every row pair.
        let itm_to_schem: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT fqn FROM objects WHERE fqn LIKE 'itm.schem.%' AND kind = 'Item' AND is_canonical = 1",
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|itm_fqn| {
                    let schem_fqn = itm_fqn.replacen("itm.schem.", "schem.", 1);
                    schem_payloads.get(&schem_fqn).map(|p| (itm_fqn, p.clone()))
                })
                .collect();
            collected
        };

        let tx = conn.unchecked_transaction()?;
        let mut schem_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO schematics (schematic_fqn, output_fqn, output_resolved) \
             VALUES (?1, ?2, ?3)",
        )?;
        let mut mat_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO schematic_materials (schematic_fqn, material_fqn, quantity) \
             VALUES (?1, ?2, ?3)",
        )?;

        let mut count = 0u64;
        for (schematic_fqn, payload_b64) in &itm_to_schem {
            let Ok(payload) = BASE64.decode(payload_b64) else {
                continue;
            };

            let mut output_fqn: Option<String> = None;
            let mut materials: Vec<(String, u32)> = Vec::new();

            let mut i = 0;
            while i + 10 <= payload.len() {
                if payload[i] == 0xCF && payload[i + 1] == 0xE0 {
                    let ref_guid: String = payload[i + 1..i + 9]
                        .iter()
                        .map(|b| format!("{:02X}", b))
                        .collect();
                    let qty_byte = payload[i + 9];
                    if let Some(fqn) = guid_to_fqn.get(&ref_guid) {
                        if fqn.starts_with("itm.mat.") {
                            // Quantity follows the 9-byte CF marker. Reject
                            // values >99 to avoid mistaking a continuation
                            // byte for a quantity.
                            let qty = if qty_byte == 0 || qty_byte > 99 {
                                1
                            } else {
                                qty_byte as u32
                            };
                            materials.push((fqn.clone(), qty));
                        } else if fqn.starts_with("itm.")
                            && !fqn.starts_with("itm.schem.")
                            && fqn != schematic_fqn
                            && output_fqn.is_none()
                        {
                            output_fqn = Some(fqn.clone());
                        }
                    }
                    i += 9;
                } else {
                    i += 1;
                }
            }

            let resolved = output_fqn.is_some() as i32;
            schem_stmt.execute(params![schematic_fqn, output_fqn, resolved])?;
            count += 1;
            for (mat_fqn, qty) in &materials {
                mat_stmt.execute(params![schematic_fqn, mat_fqn, qty])?;
            }
        }

        drop(schem_stmt);
        drop(mat_stmt);
        tx.commit()?;
        Ok(count)
    }

    /// Populate `schematic_details` with FQN-derived profession (#178).
    ///
    /// Walks every `schem.*` canonical object, looks for a recognized
    /// crafting profession token anywhere in the FQN, and records it.
    /// `tier` and `training_cost` remain NULL pending the per-property
    /// byte-layout decode work (the int8/16/32/enum_ref/string decode
    /// gap documented in CLAUDE.md).
    pub fn populate_schematic_details_typed(&self) -> Result<u64> {
        self.flush()?;
        let conn = self.conn.lock().unwrap();
        let fqns: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT fqn FROM objects WHERE fqn LIKE 'schem.%' AND is_canonical = 1")?;
            let collected: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };
        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO schematic_details \
               (fqn, profession, tier, training_cost) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut written = 0u64;
        for fqn in &fqns {
            let profession = profession_from_fqn(fqn);
            insert.execute(params![fqn, profession, None::<i64>, None::<i64>])?;
            written += 1;
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
    }
}

/// Recognize a SWTOR crafting profession from any segment of an FQN.
/// Returns the lowercased profession name when found, None otherwise. Used
/// by `populate_schematic_details_typed` (#178).
pub(crate) fn profession_from_fqn(fqn: &str) -> Option<String> {
    const PROFESSIONS: &[&str] = &[
        "artifice",
        "armormech",
        "armstech",
        "biochem",
        "cybertech",
        "synthweaving",
    ];
    let lower = fqn.to_lowercase();
    for prof in PROFESSIONS {
        if lower.contains(prof) {
            return Some((*prof).to_string());
        }
    }
    None
}

/// Create the schematic tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
            -- Schematic recipes (#60). Each itm.schem.* schematic has a
            -- companion schem.* GOM object whose payload encodes the recipe:
            -- output item GUID + material GUIDs with quantities. The schem.*
            -- companion is reachable via a CF GUID ref in the itm.schem.*
            -- payload. Output and materials are distinguished by the resolved
            -- FQN's prefix (itm.mat.* = material, anything else = output).
            CREATE TABLE IF NOT EXISTS schematics (
                schematic_fqn TEXT PRIMARY KEY,
                output_fqn TEXT,
                output_resolved INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS schematic_materials (
                schematic_fqn TEXT NOT NULL,
                material_fqn TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                PRIMARY KEY (schematic_fqn, material_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_schematic_materials_mat ON schematic_materials(material_fqn);
            -- Schematic typed columns (#140) -- 35 props from Schematic schema.
            CREATE TABLE IF NOT EXISTS schematic_details (
                fqn               TEXT PRIMARY KEY,
                profession        TEXT,
                tier              INTEGER,
                training_cost     INTEGER
            );
        "#,
    )?;
    Ok(())
}
