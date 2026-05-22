//! Compile-time icon name overrides for abilities that don't embed icon refs in their GOM payload.

use std::collections::HashMap;

const EMBEDDED: &str = include_str!("../icon_overrides.toml");

#[derive(Debug, Default)]
pub struct IconOverrides {
    map: HashMap<String, String>,
}

impl IconOverrides {
    pub fn from_embedded() -> anyhow::Result<Self> {
        Self::from_str(EMBEDDED)
    }

    fn from_str(s: &str) -> anyhow::Result<Self> {
        #[derive(serde::Deserialize)]
        struct File {
            overrides: Vec<Entry>,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            fqn: String,
            icon_name: String,
        }

        let file: File = toml::from_str(s)?;
        let map = file
            .overrides
            .into_iter()
            .map(|e| (e.fqn, e.icon_name))
            .collect();
        Ok(Self { map })
    }

    pub fn get(&self, fqn: &str) -> Option<&str> {
        self.map.get(fqn).map(String::as_str)
    }
}
