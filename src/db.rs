//! SQLite database output

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::schema::GameObject;
use crate::stb::StbEntry;

pub struct Database {
    conn: Connection,
}

pub struct Stats {
    pub quests: u64,
    pub abilities: u64,
    pub items: u64,
    pub npcs: u64,
    pub strings: u64,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to create database")?;

        // Enable WAL mode for better write performance
        conn.pragma_update(None, "journal_mode", "WAL")?;

        Ok(Self { conn })
    }

    pub fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            -- Raw game objects table (everything we extract)
            CREATE TABLE IF NOT EXISTS objects (
                guid TEXT PRIMARY KEY,
                fqn TEXT NOT NULL,
                kind TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0,
                revision INTEGER NOT NULL DEFAULT 0,
                json TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_objects_fqn ON objects(fqn);
            CREATE INDEX IF NOT EXISTS idx_objects_kind ON objects(kind);

            -- Localized strings table (from STB files)
            CREATE TABLE IF NOT EXISTS strings (
                fqn TEXT PRIMARY KEY,          -- Full FQN: "str.abl.sith_inquisitor.skill.corruption.innervate"
                locale TEXT NOT NULL,          -- Locale: "en-us"
                id1 INTEGER NOT NULL,          -- STB ID1
                id2 INTEGER NOT NULL,          -- STB ID2
                text TEXT NOT NULL,            -- Display text
                version INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_strings_locale ON strings(locale);

            -- Typed views for convenience
            CREATE VIEW IF NOT EXISTS quests AS
                SELECT * FROM objects WHERE kind = 'Quest' OR fqn LIKE 'qst.%';

            CREATE VIEW IF NOT EXISTS abilities AS
                SELECT * FROM objects WHERE kind = 'Ability' OR fqn LIKE 'abl.%';

            CREATE VIEW IF NOT EXISTS items AS
                SELECT * FROM objects WHERE kind = 'Item' OR fqn LIKE 'itm.%';

            CREATE VIEW IF NOT EXISTS npcs AS
                SELECT * FROM objects WHERE kind = 'Npc' OR fqn LIKE 'npc.%';

            -- Extraction metadata
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;

        Ok(())
    }

    pub fn insert_object(&self, obj: &GameObject) -> Result<()> {
        if obj.guid.is_empty() {
            return Ok(()); // Skip objects without GUID
        }

        let json_str = serde_json::to_string(&obj.json)?;

        self.conn.execute(
            r#"
            INSERT INTO objects (guid, fqn, kind, version, revision, json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(guid) DO UPDATE SET
                fqn = excluded.fqn,
                kind = excluded.kind,
                version = excluded.version,
                revision = excluded.revision,
                json = excluded.json
            WHERE excluded.revision > objects.revision
            "#,
            params![obj.guid, obj.fqn, obj.kind, obj.version, obj.revision, json_str],
        )?;

        Ok(())
    }

    pub fn insert_string(
        &self,
        fqn: &str,
        locale: &str,
        entry: &StbEntry,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO strings (fqn, locale, id1, id2, text, version)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(fqn) DO UPDATE SET
                locale = excluded.locale,
                id1 = excluded.id1,
                id2 = excluded.id2,
                text = excluded.text,
                version = excluded.version
            WHERE excluded.version > strings.version
            "#,
            params![fqn, locale, entry.id1, entry.id2, entry.text, entry.version],
        )?;

        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let quests: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM quests", [], |row| row.get(0))?;
        let abilities: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM abilities", [], |row| row.get(0))?;
        let items: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))?;
        let npcs: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM npcs", [], |row| row.get(0))?;
        let strings: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM strings", [], |row| row.get(0))?;

        Ok(Stats {
            quests,
            abilities,
            items,
            npcs,
            strings,
        })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}
