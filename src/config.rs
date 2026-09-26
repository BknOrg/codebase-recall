//! `.code-rcl/config.toml`: per-project defaults for the flags that would
//! otherwise be retyped on every run.
//!
//! Precedence is always CLI flag > config.toml > built-in default. The built-in
//! defaults live only in the `Default` impls below, and the `resolve_*`
//! functions are the only place in the crate that implements the precedence.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::analysis::Language;
use crate::cli::GraphQuery;

pub const CONFIG_FILE: &str = "config.toml";

/// Valid values of `[precise.kotlin] server`.
pub const KOTLIN_SERVERS: [&str; 3] = ["jetbrains", "fwcd", "auto"];

/// Every language a `[sync] languages` token may name.
const ALL_LANGUAGES: [Language; 10] = [
    Language::Rust,
    Language::JavaScript,
    Language::TypeScript,
    Language::Python,
    Language::Java,
    Language::Kotlin,
    Language::Vue,
    Language::Svelte,
    Language::Go,
    Language::Toml,
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub storage: StorageConfig,
    pub sync: SyncConfig,
    pub graph: GraphConfig,
    pub precise: PreciseConfig,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            storage: StorageConfig::default(),
            sync: SyncConfig::default(),
            graph: GraphConfig::default(),
            precise: PreciseConfig::default(),
        }
    }
}

/// Accepted but not applied yet.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub backend: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: "bkndb".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    pub max_file_kb: u64,
    pub languages: Vec<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_file_kb: 512,
            languages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphConfig {
    pub min_confidence: f32,
    pub include_external: bool,
    pub max_nodes: usize,
    pub depth: u32,
    pub kinds: Vec<String>,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.4,
            include_external: false,
            max_nodes: 4000,
            depth: 2,
            kinds: ["imports", "calls", "contains", "implements"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PreciseConfig {
    pub enabled: bool,
    pub timeout_secs: u64,
    pub kotlin: KotlinConfig,
}

impl Default for PreciseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: 15,
            kotlin: KotlinConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KotlinConfig {
    pub server: String,
}

impl Default for KotlinConfig {
    fn default() -> Self {
        Self {
            server: "jetbrains".to_string(),
        }
    }
}

impl ProjectConfig {
    /// Load `<project>/.code-rcl/config.toml`. A missing file is the built-in
    /// defaults; nothing is created on disk.
    pub fn load(project: &Path) -> Result<Self> {
        let path = crate::cache::ctx_dir(project).join(CONFIG_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", path.display()));
            }
        };
        Self::from_toml_str(&text, &path)
    }

    /// Parse and validate `text`; `source` is only used in error messages.
    pub fn from_toml_str(text: &str, source: &Path) -> Result<Self> {
        let cfg: ProjectConfig =
            toml::from_str(text).with_context(|| format!("reading {}", source.display()))?;
        validate(&cfg, source)?;
        Ok(cfg)
    }
}

fn validate(cfg: &ProjectConfig, source: &Path) -> Result<()> {
    let file = source.display();

    let c = cfg.graph.min_confidence;
    if !(0.0..=1.0).contains(&c) {
        anyhow::bail!("{file}: [graph] min_confidence = {c} is out of range (expected 0.0 to 1.0)");
    }
    if cfg.sync.max_file_kb < 1 {
        anyhow::bail!(
            "{file}: [sync] max_file_kb = {} is invalid (expected an integer >= 1)",
            cfg.sync.max_file_kb
        );
    }
    let t = cfg.precise.timeout_secs;
    if !(1..=86400).contains(&t) {
        anyhow::bail!(
            "{file}: [precise] timeout_secs = {t} is out of range (expected 1 to 86400 seconds)"
        );
    }
    let server = cfg.precise.kotlin.server.as_str();
    if !KOTLIN_SERVERS.contains(&server) {
        anyhow::bail!(
            "{file}: [precise.kotlin] server = \"{server}\" is not valid (expected one of: {})",
            KOTLIN_SERVERS.join(", ")
        );
    }
    for token in &cfg.sync.languages {
        if !ALL_LANGUAGES.iter().any(|l| l.matches_filter(token)) {
            let valid: Vec<&str> = ALL_LANGUAGES.iter().map(|l| l.group()).collect();
            anyhow::bail!(
                "{file}: [sync] languages contains unknown language \"{token}\" (valid: {}; aliases js, ts, py, kt)",
                valid.join(", ")
            );
        }
    }
    Ok(())
}

/// Graph filters after applying CLI flag > config.toml > built-in default.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphSettings {
    pub min_confidence: f32,
    pub include_external: bool,
    pub max_nodes: usize,
    pub depth: u32,
    pub kinds: Vec<String>,
}

pub fn resolve_graph(query: &GraphQuery, cfg: &ProjectConfig) -> GraphSettings {
    GraphSettings {
        min_confidence: query.min_confidence.unwrap_or(cfg.graph.min_confidence),
        include_external: query.include_external.unwrap_or(cfg.graph.include_external),
        max_nodes: query.max_nodes.unwrap_or(cfg.graph.max_nodes),
        depth: query.depth.unwrap_or(cfg.graph.depth),
        kinds: if query.kinds.is_empty() {
            cfg.graph.kinds.clone()
        } else {
            query.kinds.clone()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    fn src() -> &'static Path {
        Path::new("proj/.code-rcl/config.toml")
    }

    fn parse(text: &str) -> Result<ProjectConfig> {
        ProjectConfig::from_toml_str(text, src())
    }

    fn query(args: &[&str]) -> GraphQuery {
        let mut argv = vec!["code-rcl", "graph"];
        argv.extend_from_slice(args);
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Graph(g) => g.query,
            _ => unreachable!(),
        }
    }

    #[test]
    fn empty_and_partial_configs_use_defaults() {
        assert_eq!(parse("").unwrap(), ProjectConfig::default());
        let cfg = parse("[graph]\nmin_confidence = 0.7\n").unwrap();
        assert_eq!(cfg.graph.min_confidence, 0.7);
        assert_eq!(cfg.graph.max_nodes, 4000);
        assert_eq!(cfg.sync.max_file_kb, 512);
    }

    #[test]
    fn old_init_config_still_loads() {
        let old = r#"schema_version = 1
[storage]
backend = "bkndb"
[sync]
max_file_kb = 512
languages = ["rust", "javascript", "typescript", "python", "java", "kotlin"]
[graph]
min_confidence = 0.4
include_external = false
"#;
        let cfg = parse(old).unwrap();
        assert_eq!(cfg.sync.languages.len(), 6);
    }

    #[test]
    fn unknown_keys_are_errors_naming_file_and_key() {
        for text in [
            "bogus = 1\n",
            "[graph]\nmin_confidense = 0.5\n",
            "[precise.kotlin]\nservr = \"fwcd\"\n",
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains("config.toml"), "{err}");
            let key = if text.contains("bogus") {
                "bogus"
            } else if text.contains("min_confidense") {
                "min_confidense"
            } else {
                "servr"
            };
            assert!(err.contains(key), "{err}");
        }
    }

    #[test]
    fn min_confidence_range_is_checked() {
        let err = format!(
            "{:#}",
            parse("[graph]\nmin_confidence = 1.5\n").unwrap_err()
        );
        assert!(err.contains("1.5") && err.contains("0.0 to 1.0"), "{err}");
        assert!(parse("[graph]\nmin_confidence = -0.1\n").is_err());
    }

    #[test]
    fn invalid_values_are_errors() {
        let err = format!(
            "{:#}",
            parse("[precise.kotlin]\nserver = \"jetbrain\"\n").unwrap_err()
        );
        assert!(err.contains("jetbrain"), "{err}");
        assert!(err.contains("jetbrains, fwcd, auto"), "{err}");
        assert!(parse("[precise]\ntimeout_secs = 0\n").is_err());
        assert!(parse("[precise]\ntimeout_secs = 86401\n").is_err());
        assert!(parse("[sync]\nmax_file_kb = 0\n").is_err());
        let err = format!(
            "{:#}",
            parse("[sync]\nlanguages = [\"rustt\"]\n").unwrap_err()
        );
        assert!(err.contains("rustt") && err.contains("rust"), "{err}");
        assert!(parse("[sync]\nlanguages = [\"rust\", \"py\"]\n").is_ok());
    }

    #[test]
    fn broken_toml_names_the_file() {
        let err = format!(
            "{:#}",
            parse("[graph]\nkinds = \"unterminated\n").unwrap_err()
        );
        assert!(
            err.contains("reading") && err.contains("config.toml"),
            "{err}"
        );
    }

    #[test]
    fn load_without_a_file_is_default_and_creates_nothing() {
        let dir = std::env::temp_dir().join("code_rcl_config_load_missing_project");
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = ProjectConfig::load(&dir).unwrap();
        assert_eq!(cfg, ProjectConfig::default());
        assert!(!dir.exists());
    }

    #[test]
    fn graph_flags_parse_tri_state() {
        assert_eq!(query(&[]).include_external, None);
        assert_eq!(query(&["--include-external"]).include_external, Some(true));
        assert_eq!(
            query(&["--include-external=false"]).include_external,
            Some(false)
        );
        assert_eq!(query(&[]).min_confidence, None);
        assert!(query(&[]).kinds.is_empty());
    }

    #[test]
    fn resolve_graph_precedence() {
        let mut cfg = ProjectConfig::default();
        cfg.graph.min_confidence = 0.7;
        assert_eq!(resolve_graph(&query(&[]), &cfg).min_confidence, 0.7);
        assert_eq!(
            resolve_graph(&query(&["--min-confidence", "0.2"]), &cfg).min_confidence,
            0.2
        );

        cfg.graph.include_external = true;
        assert!(resolve_graph(&query(&[]), &cfg).include_external);
        assert!(!resolve_graph(&query(&["--include-external=false"]), &cfg).include_external);
        let dflt = ProjectConfig::default();
        assert!(resolve_graph(&query(&["--include-external"]), &dflt).include_external);
        assert!(!resolve_graph(&query(&[]), &dflt).include_external);

        assert_eq!(
            resolve_graph(&query(&["--max-nodes", "0"]), &dflt).max_nodes,
            0
        );
        assert_eq!(resolve_graph(&query(&[]), &dflt).max_nodes, 4000);

        assert_eq!(
            resolve_graph(&query(&["--kinds", "calls"]), &dflt).kinds,
            vec!["calls".to_string()]
        );
        assert_eq!(resolve_graph(&query(&[]), &dflt).kinds, dflt.graph.kinds);

        cfg.graph.depth = 5;
        assert_eq!(resolve_graph(&query(&[]), &cfg).depth, 5);
        assert_eq!(resolve_graph(&query(&["--depth", "1"]), &cfg).depth, 1);
    }
}
