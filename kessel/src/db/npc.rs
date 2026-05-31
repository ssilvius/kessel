//! NPC, companion, origin, and combat-style extraction.

use super::*;

impl Database {
    /// Populate `npc_details` with typed fields decoded from NPC payloads
    /// (#176).
    ///
    /// Extracts three real values per NPC:
    ///   - `class_role`: the human-readable role label (e.g. `Humanoid - Ambient`,
    ///     `Droid - Assassin`, `Creature - Default`) which appears as the first
    ///     `"<Title> - <Subtype>"` ASCII string in the payload
    ///   - `ai_template`: the `pkg.aggro.<...>` template path the AI uses
    ///   - `faction`: FQN-derived (alliance / imperial / republic) -- NULL when
    ///     the FQN doesn't carry a clear faction segment
    ///
    /// `difficulty` and `level` remain NULL pending the per-property byte
    /// decode work that's deferred across multiple typed-detail PRs (it is
    /// the same int8/16/32/enum_ref/string decode gap documented in
    /// CLAUDE.md and quest_details_typed's foundation pass).
    ///
    /// Returns the number of npc_details rows written.
    pub fn populate_npc_details_typed(&self) -> Result<u64> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

        self.flush()?;
        let conn = self.conn.lock().unwrap();
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT fqn, json_extract(json, '$.payload_b64') \
                 FROM objects WHERE kind = 'Npc' AND is_canonical = 1",
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        const NPC_CLASS_TYPE_HI32: u32 = 0x0078E1BD;

        let tx = conn.unchecked_transaction()?;
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO npc_details \
               (fqn, difficulty, faction, class_role, ai_template, level) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        let mut written = 0u64;
        for (fqn, b64) in &rows {
            let Ok(payload) = BASE64.decode(b64) else {
                continue;
            };
            let strings = extract_ascii_strings(&payload, 4);
            let ai_template = strings
                .iter()
                .find(|s| s.starts_with("pkg.aggro."))
                .cloned();
            let class_role = strings
                .iter()
                .find(|s| s.len() < 40 && s.contains(" - ") && s.as_bytes()[0].is_ascii_uppercase())
                .cloned();
            let faction = faction_from_fqn(fqn);

            // Typed-value decode via schema-aware walker. Difficulty +
            // level come from CF40 markers whose enum/int values are now
            // surfaced (PR: wire-typed-value-decoder). Best-effort name
            // matches against the schema-derived property labels.
            let (difficulty, level) =
                match decode_payload_schema_aware(&payload, NPC_CLASS_TYPE_HI32) {
                    Ok(decoded) => {
                        let named = decoded.named_props.as_object();
                        let enum_member = |needle: &str| -> Option<String> {
                            let m = named?;
                            let (_k, v) = m.iter().find(|(k, _)| k.contains(needle))?;
                            v.get("name").and_then(|n| n.as_str()).map(String::from)
                        };
                        let int_value = |needle: &str| -> Option<i64> {
                            let m = named?;
                            let (_k, v) = m.iter().find(|(k, _)| k.contains(needle))?;
                            v.as_i64()
                        };
                        let diff = enum_member("Difficulty").or_else(|| enum_member("difficulty"));
                        let lvl = int_value("Level").or_else(|| int_value("level"));
                        (diff, lvl)
                    }
                    Err(_) => (None, None),
                };

            insert.execute(params![
                fqn,
                difficulty,
                faction,
                class_role,
                ai_template,
                level,
            ])?;
            written += 1;
        }
        drop(insert);
        tx.commit()?;
        Ok(written)
    }

    /// Populate `gsf_crew` from the `scffCrewPrototype` singleton. Each crew
    /// member is an `spvp_Crew_icon_<name>` resource string followed by its
    /// idle-animation reference. The bare `crew_name` is the icon string with
    /// the `spvp_Crew_icon_` prefix stripped. Returns rows inserted.
    pub fn populate_gsf_crew(&self) -> Result<u64> {
        self.flush()?;
        const ICON_PREFIX: &str = "spvp_Crew_icon_";
        let Some(payload) = self.load_singleton_payload("scffCrewPrototype") else {
            return Ok(0);
        };

        // Strings in payload order. A crew record is an icon string; its idle
        // animation is the next string, unless that next string is itself the
        // start of another crew record.
        let strings = extract_ascii_strings(&payload, 4);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO gsf_crew (ordinal, icon_name, crew_name, idle_animation) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut ordinal = 0i64;
            for (i, s) in strings.iter().enumerate() {
                let Some(crew_name) = s.strip_prefix(ICON_PREFIX) else {
                    continue;
                };
                let idle_animation = strings
                    .get(i + 1)
                    .filter(|next| !next.starts_with(ICON_PREFIX));
                insert.execute(params![ordinal, s, crew_name, idle_animation])?;
                ordinal += 1;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `companions` from `npc.companion.*` objects. The display name
    /// is resolved from the strings table (id2 = string_id, id1 = 0, en-us).
    /// `category` is the FQN segment after `npc.companion.` (a class name for
    /// origin-class companions, or a content source otherwise); `companion_key`
    /// is the final FQN segment. Returns rows inserted.
    pub fn populate_companions(&self) -> Result<u64> {
        self.flush()?;
        const PREFIX: &str = "npc.companion.";

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0u64;
        {
            // One row per canonical npc.companion.* object, with the en-us
            // display name left-joined so unnamed companions still appear.
            let mut select = tx.prepare(
                "SELECT o.fqn, o.string_id, o.guid, s.text \
                 FROM objects o \
                 LEFT JOIN strings s \
                   ON s.id2 = o.string_id AND s.id1 = 0 AND s.locale = 'en-us' \
                 WHERE o.fqn LIKE 'npc.companion.%' AND o.is_canonical = 1",
            )?;
            let rows = select
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO companions \
                    (fqn, companion_key, name, category, string_id, guid) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (fqn, string_id, guid, name) in rows {
                let Some(tail) = fqn.strip_prefix(PREFIX) else {
                    continue;
                };
                let category = tail.split('.').next().unwrap_or(tail).to_string();
                let companion_key = tail.rsplit('.').next().unwrap_or(tail).to_string();
                insert.execute(params![fqn, companion_key, name, category, string_id, guid])?;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Populate `origins` (#94). 8 rows, hardcoded from canonical PLAYER_CLASSES.
    /// Faction + attack_type are intrinsic to each origin and don't change with
    /// patches. `string_id` is left NULL until the canonical name source is
    /// confirmed (see follow-up on #94).
    pub fn populate_origins(&self) -> Result<u64> {
        self.flush()?;

        // (fqn_segment, faction, attack_type)
        const ORIGINS: &[(&str, &str, &str)] = &[
            ("sith_warrior", "empire", "force"),
            ("sith_inquisitor", "empire", "force"),
            ("bounty_hunter", "empire", "tech"),
            ("agent", "empire", "tech"),
            ("jedi_knight", "republic", "force"),
            ("jedi_consular", "republic", "force"),
            ("trooper", "republic", "tech"),
            ("smuggler", "republic", "tech"),
        ];

        // Resolve canonical display name via `cdx.game_rules.classes.<seg>`
        // -- a clean sequential block of 8 codex strings whose FQN trailing
        // segment matches our origin codenames exactly (agent, not
        // imperial_agent; sith_inquisitor, not inquisitor). Sample row:
        //   cdx.game_rules.classes.sith_warrior -> string_id 571462 -> "Sith Warrior".
        let cdx_strings: std::collections::HashMap<String, i64> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, string_id FROM objects \
                 WHERE fqn LIKE 'cdx.game_rules.classes.%' AND is_canonical = 1 \
                   AND string_id IS NOT NULL",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter()
                .filter_map(|(fqn, sid)| {
                    fqn.strip_prefix("cdx.game_rules.classes.")
                        .map(|seg| (seg.to_string(), sid))
                })
                .collect()
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO origins (fqn_segment, faction, attack_type, string_id) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (seg, faction, atk) in ORIGINS {
                let sid = cdx_strings.get(*seg).copied();
                stmt.execute(params![seg, faction, atk, sid])?;
            }
        }
        tx.commit()?;
        Ok(ORIGINS.len() as u64)
    }

    /// Populate `combat_styles` (#94) from `class.pc.advanced.*` GOM objects.
    /// Resolves the localized display name via `cdx.advanced_classes.<display>`,
    /// where `<display>` is the canonical name (matches the internal segment for
    /// most styles, but `force_wizard` -> `jedi_sage` and `specialist` ->
    /// `vanguard`). Some other internal names map to prefixed cdx names:
    /// `assassin` -> `sith_assassin`, `shadow` -> `jedi_shadow`, `sorcerer` ->
    /// `sith_sorcerer`.
    pub fn populate_combat_styles(&self) -> Result<u64> {
        self.flush()?;

        // Internal segment -> canonical display segment used by cdx.advanced_classes.*.
        // Default mapping is identity; this table covers only the divergent ones.
        fn display_for(seg: &str) -> &str {
            match seg {
                "force_wizard" => "jedi_sage",
                "specialist" => "vanguard",
                "assassin" => "sith_assassin",
                "shadow" => "jedi_shadow",
                "sorcerer" => "sith_sorcerer",
                other => other,
            }
        }

        // Force-vs-tech split. Cleaner here than in the cdx pass since
        // pre-7.0 advanced-class identity is what determines pool, not
        // the codex name.
        fn attack_type(seg: &str) -> &'static str {
            match seg {
                "assassin" | "shadow" | "sorcerer" | "force_wizard" | "juggernaut" | "guardian"
                | "marauder" | "sentinel" => "force",
                "powertech" | "vanguard" | "specialist" | "mercenary" | "commando"
                | "operative" | "scoundrel" | "sniper" | "gunslinger" => "tech",
                _ => "unknown",
            }
        }

        // Legacy adv-class faction. Post-7.0 origin/style decoupling does not
        // change which faction's silhouette/animations a style ships with --
        // juggernauts are still empire-coded, guardians republic-coded.
        // huttspawn nav and color tokens are strictly bipartite on this.
        fn faction(seg: &str) -> &'static str {
            match seg {
                "juggernaut" | "marauder" | "assassin" | "sorcerer" | "powertech" | "mercenary"
                | "operative" | "sniper" => "empire",
                "guardian" | "sentinel" | "shadow" | "force_wizard" | "specialist" | "commando"
                | "scoundrel" | "gunslinger" => "republic",
                _ => "unknown",
            }
        }

        // Load class.pc.advanced.* canonical rows (the 16 combat-style objects).
        let combat_objects: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT game_id, fqn FROM objects \
                 WHERE fqn LIKE 'class.pc.advanced.%' AND is_canonical = 1",
            )?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        // Load cdx.advanced_classes.<display> -> string_id map.
        let cdx_strings: std::collections::HashMap<String, i64> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fqn, string_id FROM objects \
                 WHERE fqn LIKE 'cdx.advanced_classes.%' AND is_canonical = 1 \
                   AND string_id IS NOT NULL",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter()
                .filter_map(|(fqn, sid)| {
                    fqn.strip_prefix("cdx.advanced_classes.")
                        .map(|seg| (seg.to_string(), sid))
                })
                .collect()
        };

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0u64;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO combat_styles \
                   (combat_style_id, fqn, fqn_segment, display_segment, faction, attack_type, string_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (game_id, fqn) in &combat_objects {
                let Some(seg) = fqn.strip_prefix("class.pc.advanced.") else {
                    continue;
                };
                let display = display_for(seg);
                let atk = attack_type(seg);
                let fac = faction(seg);
                let sid = cdx_strings.get(display).copied();
                stmt.execute(params![game_id, fqn, seg, display, fac, atk, sid])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }
}

/// Best-effort faction inference from FQN structure. Returns Some when a
/// clear faction segment is present, None otherwise. Used by
/// `populate_npc_details_typed` (#176).
pub(crate) fn faction_from_fqn(fqn: &str) -> Option<String> {
    let lower = fqn.to_lowercase();
    if lower.contains(".alliance.") {
        Some("alliance".into())
    } else if lower.contains(".imperial.") || lower.contains(".imp.") {
        Some("imperial".into())
    } else if lower.contains(".republic.") || lower.contains(".rep.") {
        Some("republic".into())
    } else if lower.contains(".sith_warrior.")
        || lower.contains(".sith_inquisitor.")
        || lower.contains(".bounty_hunter.")
        || lower.contains(".agent.")
    {
        Some("imperial".into())
    } else if lower.contains(".jedi_knight.")
        || lower.contains(".jedi_consular.")
        || lower.contains(".trooper.")
        || lower.contains(".smuggler.")
    {
        Some("republic".into())
    } else {
        None
    }
}

/// Create the npc tables (idempotent).
pub(crate) fn create_tables(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        r#"
            -- GSF crew roster from the scffCrewPrototype singleton. One row
            -- per crew member: the icon resource name (spvp_Crew_icon_<name>),
            -- the bare crew_name (icon prefix stripped), and the idle
            -- animation reference that follows it. Self-validating via the
            -- companion names (risha, treek, zenith, dr_eckard_lokin, ...).
            CREATE TABLE IF NOT EXISTS gsf_crew (
                ordinal        INTEGER PRIMARY KEY,
                icon_name      TEXT NOT NULL,
                crew_name      TEXT NOT NULL,
                idle_animation TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_gsf_crew_name ON gsf_crew(crew_name);
            -- Companion roster from npc.companion.* objects. The canonical
            -- list of companions with display names resolved from the strings
            -- table (id2 = string_id, id1 = 0, en-us). category is the FQN
            -- segment after `npc.companion.` -- a class name (smuggler,
            -- jedi_knight, sith_warrior, bounty_hunter, spy, sith_sorcerer,
            -- ...) for origin-class companions, or a content source (alliance,
            -- mtx, kotet, kotfe, galactic_seasons, ...) otherwise.
            -- Informational source for downstream guides; supersedes the
            -- scattered companion references in other tables.
            CREATE TABLE IF NOT EXISTS companions (
                fqn           TEXT PRIMARY KEY,
                companion_key TEXT NOT NULL,
                name          TEXT,
                category      TEXT NOT NULL,
                string_id     INTEGER,
                guid          TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_companions_category ON companions(category);
            CREATE INDEX IF NOT EXISTS idx_companions_name ON companions(name);
            -- Combat-style-level shared ability pool (#94 PR3 rework).
            -- Replaces the per-discipline fan-out previously emitted into
            -- discipline_abilities for class-shared (`abl.<origin>.<name>`),
            -- skill-utility (`abl.<origin>.skill.utility.*`), and shared-mod
            -- (`abl.<origin>.skill.mods.tierN.*`) abilities. Each origin's
            -- shared abilities fan to BOTH combat styles of that origin --
            -- e.g. Force Leap (sith_warrior class-shared) emits one row for
            -- juggernaut and one for marauder.
            --
            -- slot_type vocabulary: 'class_shared' | 'utility' | 'shared_mod'.
            -- Open TEXT (no CHECK) per D3 sign-off; expected subset documented
            -- here.
            CREATE TABLE IF NOT EXISTS combat_style_shared_abilities (
                combat_style_codename  TEXT NOT NULL,
                ability_game_id        TEXT NOT NULL,
                ability_fqn            TEXT NOT NULL,
                slot_type              TEXT NOT NULL,
                PRIMARY KEY (combat_style_codename, ability_game_id, slot_type),
                FOREIGN KEY (ability_game_id) REFERENCES objects(game_id),
                FOREIGN KEY (combat_style_codename) REFERENCES combat_styles(fqn_segment)
            );
            CREATE INDEX IF NOT EXISTS idx_combat_style_shared_abilities_style
                ON combat_style_shared_abilities(combat_style_codename);
            CREATE INDEX IF NOT EXISTS idx_combat_style_shared_abilities_abl
                ON combat_style_shared_abilities(ability_game_id);
            -- Class taxonomy (#94).
            -- Post-7.0 the system is flat: 8 origins (the legacy classes,
            -- "story" choice in character creation), 16 combat styles (the
            -- legacy advanced classes, "playstyle" choice). Eligibility =
            -- matching `attack_type` (force vs tech). No join table needed.
            --
            -- Origins have no top-level GOM object; they live as the second
            -- FQN segment in many other prefixes (`qst.class.<origin>.*`,
            -- `apc.legacy.class.<origin>.*`, etc.). Rows are derived from
            -- the canonical PLAYER_CLASSES list. `string_id` resolves via
            -- `cdx.game_rules.classes.<fqn_segment>` -- a clean 8-row codex
            -- block whose trailing FQN segment matches our origin codenames
            -- exactly (agent, sith_inquisitor, etc.).
            --
            -- Combat styles ARE GOM objects at `class.pc.advanced.<style>`.
            -- Two have internal codenames -- force_wizard = sage, specialist
            -- = vanguard. Display strings come from `cdx.advanced_classes.<style>`,
            -- which is on a different object; we resolve the cdx string_id
            -- and store it on the combat_styles row directly.
            CREATE TABLE IF NOT EXISTS origins (
                fqn_segment    TEXT PRIMARY KEY,    -- 'sith_warrior', 'agent', etc.
                faction        TEXT NOT NULL,      -- 'empire' | 'republic'
                attack_type    TEXT NOT NULL,      -- 'force' | 'tech'
                string_id      INTEGER             -- -> strings.id2 (NULL until source confirmed)
            );
            CREATE INDEX IF NOT EXISTS idx_origins_attack ON origins(attack_type);
            CREATE TABLE IF NOT EXISTS combat_styles (
                combat_style_id TEXT PRIMARY KEY,   -- game_id of class.pc.advanced.<style>
                fqn             TEXT NOT NULL UNIQUE,
                fqn_segment     TEXT NOT NULL UNIQUE,     -- internal name ('force_wizard', 'specialist', ...)
                display_segment TEXT NOT NULL,     -- canonical name ('sage', 'vanguard', ...)
                faction         TEXT NOT NULL,     -- 'empire' | 'republic' (legacy adv-class faction)
                attack_type     TEXT NOT NULL,     -- 'force' | 'tech'
                string_id       INTEGER,           -- -> strings.id2 from cdx.advanced_classes.<display>
                FOREIGN KEY (combat_style_id) REFERENCES objects(game_id)
            );
            CREATE INDEX IF NOT EXISTS idx_combat_styles_attack ON combat_styles(attack_type);
            CREATE INDEX IF NOT EXISTS idx_combat_styles_faction ON combat_styles(faction);
            CREATE INDEX IF NOT EXISTS idx_combat_styles_display ON combat_styles(display_segment);
            -- NPC typed columns (#139) -- 32 props, 5 named enums from
            -- client.gom Npc schema.
            CREATE TABLE IF NOT EXISTS npc_details (
                fqn               TEXT PRIMARY KEY,
                difficulty        TEXT,
                faction           TEXT,
                class_role        TEXT,
                ai_template       TEXT,
                level             INTEGER
            );
        "#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
    #[test]
    fn populate_origins_inserts_eight_canonical_rows() {
        let path = temp_db_path("origins");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Seed canonical-name codex rows for two origins -- the populator
        // should pick these up via cdx.game_rules.classes.<seg>. Other six
        // origins have no codex row in this fixture; their string_id should
        // remain NULL (graceful missing-source behaviour).
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('og_sw', 'sid1', 'ph1', 'guid1', 'cdx.game_rules.classes.sith_warrior', 'Codex', 571462, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('og_ag', 'sid2', 'ph2', 'guid2', 'cdx.game_rules.classes.agent', 'Codex', 571473, '{}')",
                [],
            ).unwrap();
        }

        let count = db.populate_origins().unwrap();
        assert_eq!(count, 8);

        let conn = db.conn.lock().unwrap();
        let total: u64 = conn
            .query_row("SELECT COUNT(*) FROM origins", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 8);

        // Faction split: 4/4 between empire and republic.
        let empire: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM origins WHERE faction = 'empire'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(empire, 4);

        // Attack-type split: 4/4 force vs tech, exactly the eligibility pools.
        let force: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM origins WHERE attack_type = 'force'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(force, 4);

        // Sith warrior must be empire+force; trooper must be republic+tech.
        let (sw_faction, sw_atk): (String, String) = conn
            .query_row(
                "SELECT faction, attack_type FROM origins WHERE fqn_segment = 'sith_warrior'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(sw_faction, "empire");
        assert_eq!(sw_atk, "force");

        let (tr_faction, tr_atk): (String, String) = conn
            .query_row(
                "SELECT faction, attack_type FROM origins WHERE fqn_segment = 'trooper'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tr_faction, "republic");
        assert_eq!(tr_atk, "tech");

        // Seeded origins resolve string_id; unseeded origins stay NULL.
        let sw_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'sith_warrior'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sw_sid, Some(571462));

        let ag_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'agent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ag_sid, Some(571473));

        let tr_sid: Option<i64> = conn
            .query_row(
                "SELECT string_id FROM origins WHERE fqn_segment = 'trooper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tr_sid, None);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn populate_combat_styles_resolves_codename_and_string_id() {
        let path = temp_db_path("combat_styles");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        // Seed a class.pc.advanced.* internal-codename row and the matching
        // cdx.advanced_classes.* display row with a string_id.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
                 VALUES ('cs_force_wizard', 'sid1', 'ph1', 'guid1', 'class.pc.advanced.force_wizard', 'class', '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('cdx_sage', 'sid2', 'ph2', 'guid2', 'cdx.advanced_classes.jedi_sage', 'Codex', 351322, '{}')",
                [],
            ).unwrap();
            // A tech style with identity-mapped name + cdx string.
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, json) \
                 VALUES ('cs_powertech', 'sid3', 'ph3', 'guid3', 'class.pc.advanced.powertech', 'class', '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json) \
                 VALUES ('cdx_pt', 'sid4', 'ph4', 'guid4', 'cdx.advanced_classes.powertech', 'Codex', 351335, '{}')",
                [],
            ).unwrap();
        }

        let count = db.populate_combat_styles().unwrap();
        assert_eq!(count, 2);

        let conn = db.conn.lock().unwrap();

        // force_wizard codename resolves to display 'jedi_sage' and gets sage's string_id.
        let (display, fac, atk, sid): (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT display_segment, faction, attack_type, string_id FROM combat_styles \
                 WHERE fqn_segment = 'force_wizard'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(display, "jedi_sage");
        assert_eq!(fac, "republic");
        assert_eq!(atk, "force");
        assert_eq!(sid, Some(351322));

        // Identity-mapped style still gets its string_id.
        let (pt_display, pt_fac, pt_atk, pt_sid): (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT display_segment, faction, attack_type, string_id FROM combat_styles \
                 WHERE fqn_segment = 'powertech'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(pt_display, "powertech");
        assert_eq!(pt_fac, "empire");
        assert_eq!(pt_atk, "tech");
        assert_eq!(pt_sid, Some(351335));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
    /// Every origin in `PLAYER_ORIGINS` must yield exactly 2 combat styles --
    /// the two advanced classes that descend from that base class.
    #[test]
    fn every_player_origin_fans_to_two_combat_styles() {
        for origin in PLAYER_ORIGINS {
            let styles = origin_combat_styles(origin);
            assert_eq!(
                styles.len(),
                2,
                "origin {} should fan to 2 combat styles, got {:?}",
                origin,
                styles,
            );
        }
    }
    /// Each combat style emitted by either map must join cleanly to
    /// `combat_styles.fqn_segment` populated from `class.pc.advanced.<seg>`.
    /// The integration check uses a fixture that mirrors the live game's
    /// 16 advanced-class FQN segments.
    #[test]
    fn combat_style_values_join_combat_styles_fqn_segment() {
        // The 16 advanced-class internal codenames as they appear in
        // `class.pc.advanced.*` -- verified against live spice.sqlite.
        const ADVANCED_CLASS_FQN_SEGMENTS: &[&str] = &[
            "juggernaut",
            "marauder",
            "assassin",
            "sorcerer",
            "powertech",
            "mercenary",
            "operative",
            "sniper",
            "guardian",
            "sentinel",
            "shadow",
            "force_wizard",
            "specialist",
            "commando",
            "scoundrel",
            "gunslinger",
        ];

        for (origin, discipline, combat_style) in DISCIPLINE_COMBAT_STYLE_MAP {
            assert!(
                ADVANCED_CLASS_FQN_SEGMENTS.contains(combat_style),
                "DISCIPLINE_COMBAT_STYLE_MAP entry ({}, {}) -> '{}' is not a valid \
                 class.pc.advanced.<seg> codename. If SWTOR added/renamed an \
                 advanced class, update both this fixture AND the map.",
                origin,
                discipline,
                combat_style,
            );
        }

        for origin in PLAYER_ORIGINS {
            for combat_style in origin_combat_styles(origin) {
                assert!(
                    ADVANCED_CLASS_FQN_SEGMENTS.contains(combat_style),
                    "origin_combat_styles({}) yields '{}' which is not a valid \
                     class.pc.advanced.<seg> codename.",
                    origin,
                    combat_style,
                );
            }
        }
    }
    #[test]
    fn populate_companions_resolves_name_and_category() {
        let path = temp_db_path("companions");
        let db = Database::with_grammar(&path, None).unwrap();
        db.init_schema().unwrap();

        {
            let conn = db.conn.lock().unwrap();
            // A class companion with a name, an mtx companion without a name,
            // and a non-companion object that must be ignored.
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json, is_canonical) \
                 VALUES ('c1','s1','p1','g1','npc.companion.smuggler.corso_riggs','Npc',5001,'{}',1)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json, is_canonical) \
                 VALUES ('c2','s2','p2','g2','npc.companion.mtx.creature.akk_dog','Npc',NULL,'{}',1)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO objects (game_id, stable_id, payload_hash, guid, fqn, kind, string_id, json, is_canonical) \
                 VALUES ('n1','s3','p3','g3','npc.coruscant.guard','Npc',5001,'{}',1)",
                [],
            ).unwrap();
            // Name string: id2 = string_id, id1 = 0, en-us.
            conn.execute(
                "INSERT INTO strings (fqn, locale, id1, id2, text, version) \
                 VALUES ('str.npc','en-us',0,5001,'Corso Riggs',1)",
                [],
            )
            .unwrap();
        }

        let n = db.populate_companions().unwrap();
        assert_eq!(n, 2, "only the two npc.companion.* objects, not the guard");

        let conn = db.conn.lock().unwrap();
        let (key, name, category): (String, Option<String>, String) = conn
            .query_row(
                "SELECT companion_key, name, category FROM companions \
                 WHERE fqn = 'npc.companion.smuggler.corso_riggs'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(key, "corso_riggs");
        assert_eq!(name.as_deref(), Some("Corso Riggs"));
        assert_eq!(category, "smuggler");

        // Unnamed mtx companion: category from the segment after the prefix,
        // key from the final segment, name NULL.
        let (key2, name2, cat2): (String, Option<String>, String) = conn
            .query_row(
                "SELECT companion_key, name, category FROM companions \
                 WHERE fqn = 'npc.companion.mtx.creature.akk_dog'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(key2, "akk_dog");
        assert_eq!(name2, None);
        assert_eq!(cat2, "mtx");
    }
}
