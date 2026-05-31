//! Item classification, itemization tables, per-item stats, and relic proc classification.

use super::*;
use crate::schema::item;

impl Database {
    pub fn populate_item_tables(&self) -> Result<u64> {
        self.flush()?;

        // Fetch fqn + payload b64 for all Items. Payload is consulted only as
        // a fallback when classify() returns None for item_level (FQN had no
        // ilvl_NNNN segment). See item::extract_item_level_from_payload.
        let rows: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'Item' AND is_canonical = 1",
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0u64;

        {
            use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO item_details (fqn, item_kind, slot, weapon_type, armor_weight, rarity, item_level, source, is_schematic, crew_skill) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for (fqn, payload_b64) in &rows {
                let mut d = item::classify(fqn);
                if d.item_level.is_none() {
                    if let Ok(payload) = BASE64.decode(payload_b64) {
                        d.item_level = item::extract_item_level_from_payload(&payload);
                    }
                }
                stmt.execute(params![
                    d.fqn,
                    d.item_kind,
                    d.slot,
                    d.weapon_type,
                    d.armor_weight,
                    d.rarity,
                    d.item_level,
                    d.source,
                    if d.is_schematic { 1 } else { 0 },
                    d.crew_skill,
                ])?;
                count += 1;
            }
        }

        tx.commit()?;
        Ok(count)
    }

    /// Populate `item_sets` and `item_set_members` from `itm.setbonus.*` FQNs.
    /// Returns `(sets_count, members_count)`.
    ///
    /// Set membership is FQN-derived: the segment immediately before the
    /// trailing slot segment identifies the set. The lockbox member of each
    /// set (`<set_fqn>.armor_box`) carries the set's display name string_id,
    /// which we propagate to `item_sets.name_string_id`.
    ///
    /// Tier-bonus text (the 2/4/6-piece descriptions) is NOT included --
    /// those strings live in str.abl.* namespace and need a separate
    /// resolver pass.
    pub fn populate_item_sets(&self) -> Result<(u64, u64)> {
        self.flush()?;

        // Collect every itm.setbonus.* canonical row with its game_id +
        // string_id. Split FQN into (set_fqn, slot, source, class_group).
        let conn = self.conn.lock().unwrap();
        let rows: Vec<(String, String, Option<i64>)> = {
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn, string_id FROM objects \
                 WHERE kind='Item' AND fqn LIKE 'itm.setbonus.%' AND is_canonical=1",
            )?;
            let collected: Vec<(String, String, Option<i64>)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };
        drop(conn);

        // Build (set_fqn -> name_string_id) by finding the .armor_box (or
        // similar lockbox/box) member of each set. Falls back to None if no
        // lockbox exists -- a few sets use a different name carrier.
        let mut set_to_name_stid: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        for (_game_id, fqn, sid) in &rows {
            if !fqn.ends_with(".armor_box") && !fqn.ends_with(".weapon_box") {
                continue;
            }
            let set_fqn = match fqn.rsplit_once('.') {
                Some((set, _slot)) => set.to_string(),
                None => continue,
            };
            if let Some(s) = sid {
                set_to_name_stid.insert(set_fqn, *s);
            }
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut sets_count = 0u64;
        let mut members_count = 0u64;
        let mut seen_sets: std::collections::HashSet<String> = std::collections::HashSet::new();

        {
            let mut stmt_set = tx.prepare_cached(
                "INSERT OR IGNORE INTO item_sets (set_fqn, source, class_group, name_string_id) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut stmt_member = tx.prepare_cached(
                "INSERT OR IGNORE INTO item_set_members (item_game_id, set_fqn, slot) \
                 VALUES (?1, ?2, ?3)",
            )?;

            for (game_id, fqn, _sid) in &rows {
                let parts: Vec<&str> = fqn.split('.').collect();
                // Need: itm . setbonus . source . class_group . [optional segs] . set_id . slot
                if parts.len() < 6 {
                    continue;
                }
                let slot = *parts.last().unwrap_or(&"");
                let set_fqn = match fqn.rsplit_once('.') {
                    Some((set, _)) => set.to_string(),
                    None => continue,
                };
                let source = parts.get(2).copied().unwrap_or("").to_string();
                let class_group = parts.get(3).copied().unwrap_or("").to_string();

                if seen_sets.insert(set_fqn.clone()) {
                    let name_stid = set_to_name_stid.get(&set_fqn).copied();
                    stmt_set.execute(params![set_fqn, source, class_group, name_stid])?;
                    sets_count += 1;
                }
                stmt_member.execute(params![game_id, set_fqn, slot])?;
                members_count += 1;
            }
        }

        tx.commit()?;
        Ok((sets_count, members_count))
    }

    /// Populate the three item itemization tables from the rating, budget, and
    /// modifier-package prototype singletons. SWTOR item stats are computed
    /// from these (see the schema comment): rating from item_rating_table,
    /// the stat budget pool from item_budget_table, and the per-stat split
    /// from item_modifier_packages. Returns (rating, budget, modpkg) counts.
    ///
    /// Shapes (decoded with the typed-value GOM grammar):
    ///   itmRatings              = Map<level, Map<quality, rating>>
    ///   itmBudgetedAttributes   = Map<quality, List[level]<List[permille]<i64>>>
    ///   itmModifierPackagesList = Map<mod_id, List<{..., itmModPkgAttributePercentages: Map<stat, permille>}>>
    pub fn populate_item_itemization(&self) -> Result<(u64, u64, u64)> {
        use crate::gom_reader::{read_first_field, GomValue};
        use crate::gom_schema::{enum_member, quality_label};

        // Load all payloads before opening the transaction
        // (load_singleton_payload takes the connection lock internally).
        let rating_payload = self.load_singleton_payload("itmRatingTablePrototype");
        let budget_payload = self.load_singleton_payload("itmBudgetedAttributesPrototype");
        let modpkg_payload = self.load_singleton_payload("itmModifierPackageTablePrototype");

        self.with_tx(|tx| {
            let (mut n_rating, mut n_budget, mut n_modpkg) = (0u64, 0u64, 0u64);

            // -- Rating: Map<level, Map<quality, rating>> --
            if let Some(payload) = &rating_payload {
                let table = read_first_field(payload)?;
                tx.execute("DELETE FROM item_rating_table", [])?;
                let mut insert = tx.prepare_cached(
                    "INSERT OR REPLACE INTO item_rating_table (item_level, quality, rating) \
                 VALUES (?1, ?2, ?3)",
                )?;
                for (level_key, inner) in table.as_map().unwrap_or(&[]) {
                    let (Some(level), Some(qmap)) = (level_key.as_i64(), inner.as_map()) else {
                        continue;
                    };
                    for (q_key, r_val) in qmap {
                        let (Some(q), Some(rating)) = (q_key.as_i64(), r_val.as_i64()) else {
                            continue;
                        };
                        let Some(qname) = quality_label(q) else {
                            continue;
                        };
                        insert.execute(params![level, qname, rating])?;
                        n_rating += 1;
                    }
                }
            }

            // -- Budget: Map<quality, List[level]<List[permille]<i64>>> --
            if let Some(payload) = &budget_payload {
                let table = read_first_field(payload)?;
                tx.execute("DELETE FROM item_budget_table", [])?;
                let mut insert = tx.prepare_cached(
                    "INSERT OR REPLACE INTO item_budget_table \
                    (quality, item_level, permille, value) \
                 VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (q_key, levels) in table.as_map().unwrap_or(&[]) {
                    let (Some(q), Some(level_list)) = (q_key.as_i64(), levels.as_list()) else {
                        continue;
                    };
                    let Some(qname) = quality_label(q) else {
                        continue;
                    };
                    for (level, slots) in level_list.iter().enumerate() {
                        let Some(slot_list) = slots.as_list() else {
                            continue;
                        };
                        for (permille, val) in slot_list.iter().enumerate() {
                            let Some(value) = val.as_i64() else { continue };
                            insert.execute(params![qname, level as i64, permille as i64, value])?;
                            n_budget += 1;
                        }
                    }
                }
            }

            // -- Modifier packages: Map<mod_id, List<{..., percentages: Map<stat, permille>}>> --
            if let Some(payload) = &modpkg_payload {
                let table = read_first_field(payload)?;
                tx.execute("DELETE FROM item_modifier_packages", [])?;
                let mut insert = tx.prepare_cached(
                    "INSERT OR REPLACE INTO item_modifier_packages \
                    (mod_id, stat_index, stat_name, permille) \
                 VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (mod_key, pkg_val) in table.as_map().unwrap_or(&[]) {
                    let Some(mod_id) = mod_key.as_i64() else {
                        continue;
                    };
                    // The package value is a single-element list wrapping the
                    // package object; accept a bare object too for robustness. The
                    // stat split (itmModPkgAttributePercentages) is the package
                    // object's sole map field.
                    let pkg = match pkg_val {
                        GomValue::List(items) => items.first(),
                        other @ GomValue::Embedded(_) => Some(other),
                        _ => None,
                    };
                    let Some(pct_map) = pkg
                        .and_then(GomValue::embedded_first_map)
                        .and_then(GomValue::as_map)
                    else {
                        continue;
                    };
                    for (stat_key, pct_val) in pct_map {
                        let (Some(stat_idx), Some(permille)) =
                            (stat_key.as_i64(), pct_val.as_i64())
                        else {
                            continue;
                        };
                        let Some(sname) = enum_member("STAT", stat_idx) else {
                            continue;
                        };
                        insert.execute(params![mod_id, stat_idx, sname, permille])?;
                        n_modpkg += 1;
                    }
                }
            }

            Ok((n_rating, n_budget, n_modpkg))
        })
    }

    /// Populate `item_granted_abilities`: for each canonical `itm.*` object,
    /// decode its payload and read the granted-ability field (GOM field id
    /// low32 `0x2d7b8786`, a UInt64 object guid). Items that grant no ability
    /// (plain gear) lack the field and are skipped. The guid is resolved
    /// against `objects` for the ability FQN/kind and its `id1=1` effect text
    /// where the ability is itself extracted.
    ///
    /// Returns (rows_linked, rows_resolved_with_fqn).
    pub fn populate_item_granted_abilities(&self) -> Result<(u64, u64)> {
        use crate::gom_reader::GomValue;
        use rusqlite::OptionalExtension;
        const GRANTED_ABILITY_FIELD: u32 = 0x2d7b_8786;

        self.with_tx(|tx| {
            // Resolve a granted-ability guid to (fqn, kind, id1=1 effect text).
            let mut resolve = tx.prepare_cached(
                "SELECT o.fqn, o.kind, s.text \
                 FROM objects o \
                 LEFT JOIN strings s \
                   ON s.id2 = o.string_id AND s.id1 = 1 AND s.locale = 'en-us' \
                 WHERE o.guid = ?1 AND o.is_canonical = 1 LIMIT 1",
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO item_granted_abilities \
                    (item_fqn, item_game_id, ability_guid, ability_fqn, ability_kind, effect_text) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let (mut linked, mut resolved) = (0u64, 0u64);
            for_each_item_object(tx, |fqn, game_id, obj| {
                let Some(guid) = obj
                    .embedded_field(GRANTED_ABILITY_FIELD)
                    .and_then(GomValue::as_i64)
                else {
                    return Ok(());
                };
                // Field is a UInt64 object guid; format to match objects.guid.
                let guid_hex = format!("{:016X}", guid as u64);

                let (ability_fqn, ability_kind, effect_text) = resolve
                    .query_row([&guid_hex], |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .optional()?
                    .unwrap_or((None, None, None));

                if ability_fqn.is_some() {
                    resolved += 1;
                }
                insert.execute(params![
                    fqn,
                    game_id,
                    guid_hex,
                    ability_fqn,
                    ability_kind,
                    effect_text
                ])?;
                linked += 1;
                Ok(())
            })?;
            Ok((linked, resolved))
        })
    }

    /// Populate `item_stats`: each item's fixed stat block, decoded from the
    /// item payload's `itmEquipModStats` field (GOM field id low32 0xa4faffdd,
    /// a `Map<STAT-enum, value>`), plus its level/quality/rating metadata. This
    /// is the per-item stat display path. Items without the field (moddable
    /// shells, materials, etc.) produce no rows. Returns (items, stat rows).
    pub fn populate_item_stats(&self) -> Result<(u64, u64)> {
        use crate::gom_reader::GomValue;
        use crate::gom_schema::{enum_member, quality_label};
        const EQUIP_MOD_STATS: u32 = 0xa4fa_ffdd;
        const BASE_LEVEL: u32 = 0xc7c4_8e7c;
        const BASE_QUALITY: u32 = 0xc7c4_8e7d;
        const RATING: u32 = 0x191f_29c8;

        self.with_tx(|tx| {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO item_stats \
                    (item_fqn, item_game_id, item_level, quality, rating, \
                     stat_index, stat_name, value) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let (mut n_items, mut n_rows, mut unknown_stats) = (0u64, 0u64, 0u64);
            for_each_item_object(tx, |fqn, game_id, obj| {
                // Only items with a fixed stat block produce rows.
                let Some(stats) = obj
                    .embedded_field(EQUIP_MOD_STATS)
                    .and_then(GomValue::as_map)
                else {
                    return Ok(());
                };
                let level = obj.embedded_field(BASE_LEVEL).and_then(GomValue::as_i64);
                let quality = obj
                    .embedded_field(BASE_QUALITY)
                    .and_then(GomValue::as_i64)
                    .and_then(quality_label);
                let rating = obj.embedded_field(RATING).and_then(GomValue::as_i64);

                let mut wrote = false;
                for (stat_key, val) in stats {
                    let (Some(stat_idx), Some(value)) = (stat_key.as_i64(), val.as_i64()) else {
                        continue;
                    };
                    // A present stat whose index has no STAT enum member is a
                    // schema-drift signal (a stat added by a patch), not a
                    // missing-field skip -- count it so it isn't lost silently.
                    let Some(sname) = enum_member("STAT", stat_idx) else {
                        unknown_stats += 1;
                        continue;
                    };
                    insert.execute(params![
                        fqn, game_id, level, quality, rating, stat_idx, sname, value
                    ])?;
                    n_rows += 1;
                    wrote = true;
                }
                if wrote {
                    n_items += 1;
                }
                Ok(())
            })?;
            if unknown_stats > 0 {
                tracing::warn!(
                    "item_stats: {unknown_stats} stat entries had an index with no STAT enum \
                     member (schema drift?) and were skipped"
                );
            }
            Ok((n_items, n_rows))
        })
    }

    /// Populate `relic_procs`: classify each relic item by its trigger
    /// (passive `proc` vs activated `onuse`) and the stat it affects, from the
    /// relic FQN, joined to its granted-ability guid (from
    /// `item_granted_abilities`). Deterministic classification; the live proc
    /// magnitude/duration/ICD are runtime-computed and intentionally not
    /// modeled here (see the table comment). Returns (rows, classified_stat).
    pub fn populate_relic_procs(&self) -> Result<(u64, u64)> {
        self.with_tx(|tx| {
            let (mut rows, mut classified, mut unclassified) = (0u64, 0u64, 0u64);
            let mut select = tx.prepare(
                "SELECT o.fqn, o.game_id, ga.ability_guid \
                 FROM objects o \
                 JOIN item_details d ON d.fqn = o.fqn \
                 LEFT JOIN item_granted_abilities ga ON ga.item_fqn = o.fqn \
                 WHERE d.slot = 'relic' AND o.is_canonical = 1",
            )?;
            let relics = select
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO relic_procs \
                    (relic_fqn, relic_game_id, trigger_kind, proc_stat, proc_ability_guid) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (fqn, game_id, guid) in relics {
                let trigger = relic_trigger_kind(&fqn);
                let stat = relic_proc_stat(&fqn);
                if stat.is_some() {
                    classified += 1;
                }
                // A relic the FQN classifier understood NEITHER way is a real
                // gap (new naming convention / unknown family), not a partial
                // classification -- surface it rather than store it silently.
                if trigger.is_none() && stat.is_none() {
                    unclassified += 1;
                }
                insert.execute(params![fqn, game_id, trigger, stat, guid])?;
                rows += 1;
            }
            if unclassified > 0 {
                tracing::warn!(
                    "relic_procs: {unclassified} of {rows} relics classified as neither \
                     proc nor onuse and no stat (FQN naming the classifier doesn't cover)"
                );
            }
            Ok((rows, classified))
        })
    }
}

/// Classify the stat a relic's proc/onuse affects from its FQN. Order matters:
/// the more specific tokens are checked first (e.g. `primary_stat` before the
/// generic `power`/`crit`). Returns `None` for relics with no recognized stat
/// token (cosmetic / MTX / quest relics). Pure -- unit-tested.
pub(crate) fn relic_proc_stat(fqn: &str) -> Option<&'static str> {
    if fqn.contains("primary_stat") {
        Some("mastery")
    } else if fqn.contains("alacrity") {
        Some("alacrity")
    } else if fqn.contains("shield") {
        Some("absorb")
    } else if fqn.contains("heal") {
        Some("healing")
    } else if fqn.contains("crit") {
        Some("critical")
    } else if fqn.contains("defense") {
        Some("defense")
    } else if fqn.contains("power") {
        Some("power")
    } else if fqn.contains("kinetic")
        || fqn.contains("internal")
        || fqn.contains("elemental")
        || fqn.contains("dps_energy")
    {
        Some("damage")
    } else if fqn.contains("tank") {
        Some("defense")
    } else {
        None
    }
}

/// Classify a relic's trigger from its FQN: passive `proc` vs player-activated
/// `onuse`. `None` for relics with neither token. Pure -- unit-tested.
pub(crate) fn relic_trigger_kind(fqn: &str) -> Option<&'static str> {
    if fqn.contains("onuse") {
        Some("onuse")
    } else if fqn.contains("proc") {
        Some("proc")
    } else {
        None
    }
}

/// Create the item tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
            -- Item details (classified from FQN patterns; #59).
            -- Set name and set bonus require GOM payload parsing and are
            -- deferred to a follow-up issue.
            CREATE TABLE IF NOT EXISTS item_details (
                fqn TEXT PRIMARY KEY,
                item_kind TEXT NOT NULL,
                slot TEXT,
                weapon_type TEXT,
                armor_weight TEXT,
                rarity TEXT,
                item_level INTEGER,
                source TEXT,
                is_schematic INTEGER NOT NULL DEFAULT 0,
                crew_skill TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_item_details_kind ON item_details(item_kind);
            CREATE INDEX IF NOT EXISTS idx_item_details_slot ON item_details(slot);
            CREATE INDEX IF NOT EXISTS idx_item_details_source ON item_details(source);
            CREATE INDEX IF NOT EXISTS idx_item_details_rarity ON item_details(rarity);
            CREATE INDEX IF NOT EXISTS idx_item_details_crew_skill ON item_details(crew_skill);
            -- Item itemization tables. SWTOR item stats are computed, not
            -- stored per item: an item carries only (base level, quality,
            -- modifier set id), and these three tables turn that into numbers.
            -- Decoded from the itmRatingTablePrototype /
            -- itmBudgetedAttributesPrototype / itmModifierPackageTablePrototype
            -- singletons via the typed-value GOM grammar (gom_reader).
            --
            --   rating = item_rating_table[item_level][quality]
            --   stat   = item_budget_table[quality][item_level][permille]
            -- where the item's modifier package (item_modifier_packages, keyed
            -- by itmModifierSetID) supplies the per-stat permille index into
            -- the budget curve. Oracle: budget[artifact][89] holds 484 and 167.
            CREATE TABLE IF NOT EXISTS item_rating_table (
                item_level INTEGER NOT NULL,
                quality    TEXT NOT NULL,
                rating     INTEGER NOT NULL,
                PRIMARY KEY (item_level, quality)
            );
            -- Per-quality, per-level budget curve. permille is the 0..999 slot
            -- index (0.1% steps); a modifier package picks which permille feeds
            -- each stat. ~796k rows (4 qualities x ~199 levels x 1000 slots).
            CREATE TABLE IF NOT EXISTS item_budget_table (
                quality    TEXT NOT NULL,
                item_level INTEGER NOT NULL,
                permille   INTEGER NOT NULL,
                value      INTEGER NOT NULL,
                PRIMARY KEY (quality, item_level, permille)
            );
            -- Modifier package stat split. One row per (mod_id, stat): the
            -- permille of the slot budget that stat receives (single-stat mods
            -- = 1000; a 70/30 DPS armoring = strength 700 + endurance 300).
            CREATE TABLE IF NOT EXISTS item_modifier_packages (
                mod_id     INTEGER NOT NULL,
                stat_index INTEGER NOT NULL,
                stat_name  TEXT NOT NULL,
                permille   INTEGER NOT NULL,
                PRIMARY KEY (mod_id, stat_index)
            );
            -- The ability/proc an item grants when equipped. Decoded from the
            -- item payload's granted-ability field (GOM field id low32
            -- 0x2d7b8786, a UInt64 object guid). This is the "what does this
            -- item DO" link: legendary implants -> their bonus ability (e.g.
            -- Fearless Victor), set pieces -> set-bonus abilities, relics ->
            -- their proc.
            --
            -- ability_fqn / effect_text are resolved when the granted ability
            -- is itself an extracted object (true for legendary implant and
            -- tactical abilities, abl.itm.*). Relic procs reference UNNAMED
            -- effect objects that the extraction whitelist drops, so their
            -- ability_guid is recorded but ability_fqn/effect_text are NULL --
            -- surfacing exactly which procs still need the effect-object
            -- extraction (tracked follow-up).
            CREATE TABLE IF NOT EXISTS item_granted_abilities (
                item_fqn     TEXT PRIMARY KEY,
                item_game_id TEXT,
                ability_guid TEXT NOT NULL,
                ability_fqn  TEXT,
                ability_kind TEXT,
                effect_text  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_item_granted_abilities_guid
                ON item_granted_abilities(ability_guid);
            -- Per-item stat block: the actual stats an item provides, ready for
            -- a tooltip. Stats are FIXED on equippable gear and mod pieces --
            -- stored in the item payload's itmEquipModStats field (GOM field id
            -- low32 0xa4faffdd, a Map<STAT-enum, value>).
            --
            -- Each row is one (item, stat). The item metadata (level, quality,
            -- rating) is denormalized onto every row so a tooltip is a single
            -- `WHERE item_fqn=?` with no join. Moddable shells carry no innate
            -- stats (their stats come from slotted mods, which are themselves
            -- items with their own item_stats) and so produce no rows here --
            -- that is correct, not a gap. Item stats are fixed, not derived
            -- from a per-item modifier set, so the budget/modifier-package
            -- tables are for theorycrafting, not the per-item display path.
            --   item_level = itmBaseLevel (0xc7c48e7c)
            --   quality    = itmBaseQuality (0xc7c48e7d -> itmQuality member)
            --   rating     = display item rating (0x191f29c8)
            CREATE TABLE IF NOT EXISTS item_stats (
                item_fqn     TEXT NOT NULL,
                item_game_id TEXT,
                item_level   INTEGER,
                quality      TEXT,
                rating       INTEGER,
                stat_index   INTEGER NOT NULL,
                stat_name    TEXT NOT NULL,
                value        INTEGER NOT NULL,
                PRIMARY KEY (item_fqn, stat_index)
            );
            CREATE INDEX IF NOT EXISTS idx_item_stats_rating ON item_stats(rating);
            -- Relic proc classification. One row per relic item: the trigger
            -- (passive 'proc' vs activated 'onuse') and the stat it affects,
            -- classified from the relic FQN, plus the granted-ability guid
            -- (the proc/onuse ability the relic references via field
            -- 0x2d7b8786). See proc_stat() for the full set of stat labels.
            --
            -- IMPORTANT static-data ceiling: the EXACT proc magnitude,
            -- duration, and internal cooldown are NOT in the .tor archive.
            -- Proc-ability objects are shared across rating tiers while the
            -- proc value scales with the relic's rating at runtime, so the
            -- number is computed live (the str.abl.* proc strings carry blank
            -- duration/ICD tokens). The relic's STATIC equipped stats ARE
            -- captured (in item_stats). So this table answers "what kind of
            -- relic is this" deterministically; the live proc burst value is
            -- client-side residual (cf. #111).
            CREATE TABLE IF NOT EXISTS relic_procs (
                relic_fqn         TEXT PRIMARY KEY,
                relic_game_id     TEXT,
                trigger_kind      TEXT,   -- 'proc' (passive) or 'onuse' (activated); NULL if neither
                proc_stat         TEXT,   -- see proc_stat(): power/critical/mastery/defense/healing/absorb/alacrity/damage, or NULL
                proc_ability_guid TEXT    -- granted-ability guid (item field 0x2d7b8786)
            );
            CREATE INDEX IF NOT EXISTS idx_relic_procs_stat ON relic_procs(proc_stat);
            -- Item set membership (#105 part 1).
            --
            -- Set membership is encoded in the FQN itself, not in any pkg.*
            -- mediator (the issue's original pkg.* hypothesis was disproved
            -- by Phase 1 investigation: pkg.* contains 6 profession-trainer
            -- packages, unrelated to gear sets). Pattern:
            --   itm.setbonus.<source>.<class_group>.<subclass>.<set_id>.<slot>
            -- Members sharing the leading segments through <set_id> form a set.
            --
            -- Set display name comes from the .armor_box (or similar lockbox)
            -- member, whose str.itm.0.<id> string is the set's in-game name
            -- ("Berserker's Armor Lockbox" -> the set is "Berserker's Armor").
            -- Tier-bonus text (the 2/4/6-piece descriptions) lives in
            -- str.abl.* namespace and requires a separate resolver pass; it's
            -- not in this table yet -- documented follow-up.
            CREATE TABLE IF NOT EXISTS item_sets (
                set_fqn         TEXT PRIMARY KEY,
                source          TEXT,
                class_group     TEXT,
                name_string_id  INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_item_sets_source ON item_sets(source);
            CREATE TABLE IF NOT EXISTS item_set_members (
                item_game_id    TEXT NOT NULL,
                set_fqn         TEXT NOT NULL,
                slot            TEXT NOT NULL,
                PRIMARY KEY (item_game_id, set_fqn),
                FOREIGN KEY (item_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (set_fqn) REFERENCES item_sets(set_fqn)
            );
            CREATE INDEX IF NOT EXISTS idx_item_set_members_set ON item_set_members(set_fqn);
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
    #[test]
    fn relic_classifiers_cover_real_fqn_tails() {
        // Real relic FQN tails (from the v24 extract) -> expected classification.
        // trigger: proc (passive) vs onuse (activated).
        assert_eq!(relic_trigger_kind("itm.x.relic_power_proc"), Some("proc"));
        assert_eq!(
            relic_trigger_kind("itm.x.relic_dps_power_onuse"),
            Some("onuse")
        );
        assert_eq!(relic_trigger_kind("itm.mtx.relic.bastilas_sash"), None);

        // proc_stat token mapping.
        assert_eq!(relic_proc_stat("relic_power_proc"), Some("power"));
        assert_eq!(relic_proc_stat("relic_crit_proc"), Some("critical"));
        assert_eq!(relic_proc_stat("relic_primary_stat_proc"), Some("mastery"));
        assert_eq!(relic_proc_stat("relic_heal_proc"), Some("healing"));
        assert_eq!(relic_proc_stat("relic_defense_proc"), Some("defense"));
        assert_eq!(
            relic_proc_stat("trinket_relic_static_shield_proc"),
            Some("absorb")
        );
        assert_eq!(
            relic_proc_stat("relic_dps_alacrity_onuse"),
            Some("alacrity")
        );
        assert_eq!(
            relic_proc_stat("artifact_trinket_relics_dps_kinetic_proc"),
            Some("damage")
        );
        assert_eq!(relic_proc_stat("trinket_relic_tank_proc"), Some("defense"));
        // Cosmetic / MTX relic with no recognized stat token.
        assert_eq!(relic_proc_stat("bastilas_sash"), None);

        // Precedence: a tail with both "primary_stat" and "crit" resolves to
        // the more specific primary_stat (mastery), not critical.
        assert_eq!(
            relic_proc_stat("relic_primary_stat_crit_proc"),
            Some("mastery")
        );
    }
    #[test]
    fn populate_item_sets_groups_setbonus_items_by_fqn_prefix() {
        let path = temp_db_path("item_sets");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        let set_prefix = "itm.setbonus.sow.general.offensive.berserker_rage";
        {
            let conn = db.conn.lock().unwrap();
            // Three real-shape members of the same set + a lockbox carrier.
            for (gid, slot, sid) in [
                ("g_chest", "armor_chest", 1003315),
                ("g_head", "armor_head", 1003318),
                ("g_box", "armor_box", 1096865),
            ] {
                let fqn = format!("{}.{}", set_prefix, slot);
                conn.execute(
                    "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'Item', ?6, '{}')",
                    params![gid, format!("sid_{}", gid), format!("ph_{}", gid), format!("guid_{}", gid), fqn, sid],
                ).unwrap();
            }
            // A member of a DIFFERENT set -- must be assigned its own set row.
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('g_other', 'sid_other', 'ph_other', 'guid_other', \
                         'itm.setbonus.sow.inq_con.sorc_sage.class.dmg_aoe_01.armor_head', \
                         'Item', 995377, '{}')",
                [],
            ).unwrap();
        }

        let (sets, members) = db.populate_item_sets().unwrap();
        assert_eq!(sets, 2);
        assert_eq!(members, 4);

        let conn = db.conn.lock().unwrap();

        // Berserker set carries the lockbox's string_id (1096865).
        let name_sid: Option<i64> = conn
            .query_row(
                "SELECT name_string_id FROM item_sets WHERE set_fqn=?1",
                params![set_prefix],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name_sid, Some(1096865));

        // Source + class_group derived from FQN segments.
        let (source, cgroup): (String, String) = conn
            .query_row(
                "SELECT source, class_group FROM item_sets WHERE set_fqn=?1",
                params![set_prefix],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(source, "sow");
        assert_eq!(cgroup, "general");

        // Member rows reference both the item and the set, with the slot.
        let mut slots: Vec<String> = conn
            .prepare("SELECT slot FROM item_set_members WHERE set_fqn=?1 ORDER BY slot")
            .unwrap()
            .query_map(params![set_prefix], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        slots.sort();
        assert_eq!(slots, vec!["armor_box", "armor_chest", "armor_head"]);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
