//! Grammar rules for cleaning SWTOR description templates
//!
//! SWTOR uses template syntax in descriptions:
//! - `<<N[singular/plural/plural]>>` for counts/durations
//! - `<<N>>` for damage/healing values
//!
//! This module loads rules from grammar.toml and applies them to produce
//! natural English descriptions.

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Grammar configuration loaded from TOML
#[derive(Debug, Deserialize)]
pub struct GrammarConfig {
    pub version: u32,
    #[serde(default)]
    pub templates: Vec<TemplateRule>,
    #[serde(default)]
    pub literals: Vec<LiteralRule>,
    #[serde(default)]
    pub cleanup: Vec<CleanupRule>,
}

/// Template pattern rule (for <<N[...]>> syntax)
#[derive(Debug, Deserialize)]
pub struct TemplateRule {
    pub pattern: String,
    /// Capture group index to extract (for map lookup)
    pub capture: Option<usize>,
    /// Map captured word to replacement
    #[serde(default)]
    pub map: HashMap<String, String>,
    /// Fallback if no map match
    #[serde(default)]
    pub fallback: String,
    /// Direct replacement (if no capture/map)
    pub replacement: Option<String>,
}

/// Literal string replacement
#[derive(Debug, Deserialize)]
pub struct LiteralRule {
    pub find: String,
    pub replace: String,
}

/// Cleanup regex pattern (post-processing)
#[derive(Debug, Deserialize)]
pub struct CleanupRule {
    pub pattern: String,
    pub replacement: String,
}

/// Compiled grammar rules ready for application
pub struct Grammar {
    templates: Vec<CompiledTemplate>,
    literals: Vec<LiteralRule>,
    cleanup: Vec<CompiledCleanup>,
}

struct CompiledTemplate {
    regex: Regex,
    capture: Option<usize>,
    map: HashMap<String, String>,
    fallback: String,
    replacement: Option<String>,
}

struct CompiledCleanup {
    regex: Regex,
    replacement: String,
}

/// Embedded grammar rules (compiled into binary)
const EMBEDDED_GRAMMAR: &str = include_str!("../grammar.toml");

impl Grammar {
    /// Load grammar rules embedded at compile time
    pub fn from_embedded() -> Result<Self> {
        let config: GrammarConfig = toml::from_str(EMBEDDED_GRAMMAR)
            .context("Failed to parse embedded grammar TOML")?;
        Self::from_config(config)
    }

    /// Load and compile grammar rules from TOML file
    #[allow(dead_code)]
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read grammar file: {}", path.display()))?;

        let config: GrammarConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse grammar TOML: {}", path.display()))?;

        Self::from_config(config)
    }

    /// Compile grammar rules from config
    fn from_config(config: GrammarConfig) -> Result<Self> {
        let templates = config
            .templates
            .into_iter()
            .map(|rule| {
                let regex = Regex::new(&rule.pattern)
                    .with_context(|| format!("Invalid template regex: {}", rule.pattern))?;
                Ok(CompiledTemplate {
                    regex,
                    capture: rule.capture,
                    map: rule.map,
                    fallback: rule.fallback,
                    replacement: rule.replacement,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let cleanup = config
            .cleanup
            .into_iter()
            .map(|rule| {
                let regex = Regex::new(&rule.pattern)
                    .with_context(|| format!("Invalid cleanup regex: {}", rule.pattern))?;
                Ok(CompiledCleanup {
                    regex,
                    replacement: rule.replacement,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            templates,
            literals: config.literals,
            cleanup,
        })
    }

    /// Create a no-op grammar (for when config file is missing)
    pub fn disabled() -> Self {
        Self {
            templates: vec![],
            literals: vec![],
            cleanup: vec![],
        }
    }

    /// Apply all grammar rules to a string
    pub fn clean(&self, text: &str) -> String {
        let mut result = text.to_string();

        // 1. Apply template rules
        for template in &self.templates {
            result = self.apply_template(template, &result);
        }

        // 2. Apply literal replacements
        for literal in &self.literals {
            result = result.replace(&literal.find, &literal.replace);
        }

        // 3. Apply cleanup regexes
        for cleanup in &self.cleanup {
            result = cleanup
                .regex
                .replace_all(&result, &cleanup.replacement)
                .to_string();
        }

        result.trim().to_string()
    }

    fn apply_template(&self, template: &CompiledTemplate, text: &str) -> String {
        // Direct replacement (no capture group)
        if let Some(ref replacement) = template.replacement {
            return template.regex.replace_all(text, replacement).to_string();
        }

        // Capture group with map lookup
        if let Some(capture_idx) = template.capture {
            let mut result = text.to_string();

            // Find all matches and replace
            while let Some(caps) = template.regex.captures(&result) {
                let full_match = caps.get(0).unwrap();
                let captured = caps
                    .get(capture_idx)
                    .map(|m| m.as_str().to_lowercase())
                    .unwrap_or_default();

                let replacement = template
                    .map
                    .get(&captured)
                    .cloned()
                    .unwrap_or_else(|| template.fallback.clone());

                // Build new string with replacement
                let before = &result[..full_match.start()];
                let after = &result[full_match.end()..];
                result = format!("{}{}{}", before, replacement, after);
            }

            return result;
        }

        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grammar() -> Grammar {
        let config = GrammarConfig {
            version: 1,
            templates: vec![
                TemplateRule {
                    pattern: r"<<\d+\[%d\s+(\w+)[^\]]*\]>>".to_string(),
                    capture: Some(1),
                    map: [
                        ("seconds".to_string(), "time".to_string()),
                        ("second".to_string(), "time".to_string()),
                    ]
                    .into(),
                    fallback: String::new(),
                    replacement: None,
                },
                TemplateRule {
                    pattern: r"<<\d+>>".to_string(),
                    capture: None,
                    map: HashMap::new(),
                    fallback: String::new(),
                    replacement: Some(String::new()),
                },
            ],
            literals: vec![LiteralRule {
                find: " an additional ".to_string(),
                replace: " additional ".to_string(),
            }],
            cleanup: vec![CleanupRule {
                pattern: r"\s{2,}".to_string(),
                replacement: " ".to_string(),
            }],
        };
        Grammar::from_config(config).unwrap()
    }

    #[test]
    fn test_duration_template() {
        let grammar = test_grammar();
        let input = "slows the target over <<1[%d seconds/%d second/%d seconds]>>";
        let result = grammar.clean(input);
        assert_eq!(result, "slows the target over time");
    }

    #[test]
    fn test_value_placeholder() {
        let grammar = test_grammar();
        let input = "deals <<3>> kinetic damage";
        let result = grammar.clean(input);
        assert_eq!(result, "deals kinetic damage");
    }

    #[test]
    fn test_article_cleanup() {
        let grammar = test_grammar();
        let input = "takes an additional <<2>> damage";
        let result = grammar.clean(input);
        assert_eq!(result, "takes additional damage");
    }

    #[test]
    fn test_force_exhaustion() {
        let grammar = test_grammar();
        let input = "Progressively slows the target from 50% to 5% movement speed over <<1[%d seconds/%d second/%d seconds]>> and deals <<3>> kinetic damage each second. At the end of the duration, the target is crushed and takes an additional <<2>> kinetic damage.";
        let result = grammar.clean(input);
        assert_eq!(result, "Progressively slows the target from 50% to 5% movement speed over time and deals kinetic damage each second. At the end of the duration, the target is crushed and takes additional kinetic damage.");
    }
}
