use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

pub const DEFAULT_SIZE_WARN: u64 = 400;
pub const DEFAULT_SIZE_STRONG_WARN: u64 = 800;
pub const DEFAULT_MAX_FILES: usize = 15;
pub const DEFAULT_MAX_DIRS: usize = 5;

pub fn default_risky_keywords() -> Vec<String> {
    [
        "auth",
        "login",
        "session",
        "payment",
        "billing",
        "migration",
        "security",
        "secret",
        ".env",
        "permission",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub base_branch: Option<String>,
    pub size_warn: u64,
    pub size_strong_warn: u64,
    pub max_files: usize,
    pub max_dirs: usize,
    pub risky_keywords: Vec<String>,
    pub ignore_patterns: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_branch: None,
            size_warn: DEFAULT_SIZE_WARN,
            size_strong_warn: DEFAULT_SIZE_STRONG_WARN,
            max_files: DEFAULT_MAX_FILES,
            max_dirs: DEFAULT_MAX_DIRS,
            risky_keywords: default_risky_keywords(),
            ignore_patterns: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    base_branch: Option<String>,
    size_warn: Option<u64>,
    size_strong_warn: Option<u64>,
    max_files: Option<usize>,
    max_dirs: Option<usize>,
    risky_keywords: Option<Vec<String>>,
    ignore_patterns: Option<Vec<String>>,
}

pub fn parse_toml(content: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(content).context("Failed to parse .prsherpa.toml")?;
    let defaults = Config::default();
    Ok(Config {
        base_branch: raw.base_branch,
        size_warn: raw.size_warn.unwrap_or(defaults.size_warn),
        size_strong_warn: raw.size_strong_warn.unwrap_or(defaults.size_strong_warn),
        max_files: raw.max_files.unwrap_or(defaults.max_files),
        max_dirs: raw.max_dirs.unwrap_or(defaults.max_dirs),
        risky_keywords: raw.risky_keywords.unwrap_or(defaults.risky_keywords),
        ignore_patterns: raw.ignore_patterns.unwrap_or(defaults.ignore_patterns),
    })
}

/// Loads config from an explicit `--config` path, or `.prsherpa.toml` at `repo_root`.
/// Missing default `.prsherpa.toml` silently falls back to defaults.
pub fn load_config(explicit_path: Option<&Path>, repo_root: Option<&Path>) -> Result<Config> {
    if let Some(path) = explicit_path {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file '{}'.", path.display()))?;
        return parse_toml(&content);
    }

    if let Some(root) = repo_root {
        let candidate = root.join(".prsherpa.toml");
        if candidate.is_file() {
            let content = fs::read_to_string(&candidate).with_context(|| {
                format!("Failed to read config file '{}'.", candidate.display())
            })?;
            return parse_toml(&content);
        }
    }

    Ok(Config::default())
}

pub fn matches_ignore_pattern(path: &str, patterns: &[String]) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    patterns
        .iter()
        .any(|pat| simple_glob_match(&pat.replace('\\', "/").to_ascii_lowercase(), &normalized))
}

fn simple_glob_match(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return text == pattern
            || text.starts_with(&format!("{pattern}/"))
            || text.contains(&format!("/{pattern}/"));
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut remainder = text;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 {
            if !remainder.starts_with(part) {
                return false;
            }
            remainder = &remainder[part.len()..];
        } else if idx == parts.len() - 1 && !pattern.ends_with('*') {
            return remainder.ends_with(part);
        } else if let Some(pos) = remainder.find(part) {
            remainder = &remainder[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_partial_toml_with_defaults() {
        let toml_str = r#"
base_branch = "develop"
size_warn = 300
ignore_patterns = ["*.snap", "vendor/*"]
"#;
        let cfg = parse_toml(toml_str).unwrap();
        assert_eq!(cfg.base_branch.as_deref(), Some("develop"));
        assert_eq!(cfg.size_warn, 300);
        assert_eq!(cfg.size_strong_warn, DEFAULT_SIZE_STRONG_WARN);
        assert_eq!(cfg.max_files, DEFAULT_MAX_FILES);
        assert_eq!(cfg.ignore_patterns, vec!["*.snap", "vendor/*"]);
    }

    #[test]
    fn matches_ignore_glob_patterns() {
        let patterns = vec!["*.snap".to_string(), "vendor/*".to_string()];
        assert!(matches_ignore_pattern("tests/ui.snap", &patterns));
        assert!(matches_ignore_pattern("vendor/lib/foo.rs", &patterns));
        assert!(!matches_ignore_pattern("src/main.rs", &patterns));
    }
}
