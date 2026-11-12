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
use crate::cli::{GraphQuery, PreciseArgs, SyncArgs};

pub const CONFIG_FILE: &str = "config.toml";

/// Valid values of `[precise.kotlin] server`.
pub const KOTLIN_SERVERS: [&str; 3] = ["jetbrains", "fwcd", "auto"];
/// Storage engines for `[storage] backend`; bkndb is the only one.
pub const STORAGE_BACKENDS: [&str; 1] = ["bkndb"];

/// Every language a `[sync] languages` token may name.
const ALL_LANGUAGES: [Language; 12] = [
    Language::Rust,
    Language::C,
    Language::Cpp,
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
    #[serde(default)]
    pub storage: Option<StorageConfig>,
    pub sync: SyncConfig,
    pub graph: GraphConfig,
    pub precise: PreciseConfig,
    pub c_family: CFamilyConfig,
    pub embeddings: EmbeddingsConfig,
    pub scope: ScopeConfig,
    pub search: SearchConfig,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            storage: None,
            sync: SyncConfig::default(),
            graph: GraphConfig::default(),
            precise: PreciseConfig::default(),
            c_family: CFamilyConfig::default(),
            embeddings: EmbeddingsConfig::default(),
            scope: ScopeConfig::default(),
            search: SearchConfig::default(),
        }
    }
}

/// Which engine holds `.code-rcl/cache.bkndb`; see [`STORAGE_BACKENDS`].
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
            min_confidence: 0.3,
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
    pub selective: bool,
    pub selective_budget_secs: u64,
    pub timeout_secs: u64,
    pub kotlin: KotlinConfig,
    pub clangd: ClangdConfig,
    pub vue: VueConfig,
}

impl Default for PreciseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            selective: false,
            selective_budget_secs: 30,
            timeout_secs: 15,
            kotlin: KotlinConfig::default(),
            clangd: ClangdConfig::default(),
            vue: VueConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VueConfig {
    pub tsdk: Option<std::path::PathBuf>,
    pub plugin_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CFamilyConfig {
    pub compilation_database_dir: Option<std::path::PathBuf>,
    pub header_language_overrides: Vec<HeaderLanguageOverride>,
}
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderLanguageOverride {
    pub pattern: String,
    pub language: String,
}
#[derive(Debug, Clone, Default, PartialEq, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClangdConfig {
    pub query_drivers: Vec<String>,
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

    /// `[precise] enabled` acts like passing `--precise` (flag OR config).
    pub fn apply_precise_default(&self, precise: &mut PreciseArgs) {
        if !precise.requested() {
            precise.precise = self.precise.enabled;
            precise.precise_selective = self.precise.selective;
        }
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
    for rule in &cfg.c_family.header_language_overrides {
        anyhow::ensure!(
            ["c", "cpp"].contains(&rule.language.as_str()),
            "header override language must be c or cpp"
        );
        globset::Glob::new(&rule.pattern)?;
    }
    for driver in &cfg.precise.clangd.query_drivers {
        anyhow::ensure!(
            !driver.is_empty() && !driver.contains([',', '\n', '\r']),
            "query_drivers entries must be nonempty paths/globs without commas or newlines"
        );
    }
    let e = &cfg.embeddings;
    anyhow::ensure!(
        e.model == "embeddinggemma-300m-q4",
        "{file}: unsupported embedding model"
    );
    anyhow::ensure!(
        [256, 768].contains(&e.dimensions),
        "{file}: embedding dimensions must be 256 or 768"
    );
    anyhow::ensure!(
        (1..=32).contains(&e.batch_size),
        "{file}: embedding batch_size must be 1..=32"
    );
    anyhow::ensure!(
        e.threads <= 64,
        "{file}: embedding threads must be 0 (auto) or 1..=64"
    );

    for (name, globs) in &cfg.scope.categories {
        anyhow::ensure!(
            !name.trim().is_empty() && !globs.is_empty(),
            "scope category names and globs must not be empty"
        );
        crate::selection::globs(globs)?;
    }
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
    anyhow::ensure!(
        !(cfg.precise.enabled && cfg.precise.selective),
        "[precise] enabled and selective are mutually exclusive"
    );
    anyhow::ensure!(
        (1..=3600).contains(&cfg.precise.selective_budget_secs),
        "selective_budget_secs must be 1..=3600"
    );
    let t = cfg.precise.timeout_secs;
    if !(1..=86400).contains(&t) {
        anyhow::bail!(
            "{file}: [precise] timeout_secs = {t} is out of range (expected 1 to 86400 seconds)"
        );
    }
    if let Some(ref storage) = cfg.storage {
        let backend = storage.backend.as_str();
        if !STORAGE_BACKENDS.contains(&backend) {
            anyhow::bail!(
                "{file}: [storage] backend = \"{backend}\" is not valid (expected one of: {})",
                STORAGE_BACKENDS.join(", ")
            );
        }
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

/// Sync settings after applying CLI flag > config.toml > built-in default.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncSettings {
    pub max_bytes: u64,
    pub languages: Vec<String>,
    /// True when the language filter came from config.toml (not `--language`).
    pub languages_from_config: bool,
}

pub fn resolve_sync(args: &SyncArgs, cfg: &ProjectConfig) -> SyncSettings {
    let (languages, languages_from_config) = if !args.language.is_empty() {
        (args.language.clone(), false)
    } else {
        let from_cfg = !cfg.sync.languages.is_empty();
        (cfg.sync.languages.clone(), from_cfg)
    };
    SyncSettings {
        max_bytes: args
            .max_file_kb
            .unwrap_or(cfg.sync.max_file_kb)
            .saturating_mul(1024),
        languages,
        languages_from_config,
    }
}

pub fn resolve_precise_timeout(args: &PreciseArgs, cfg: &ProjectConfig) -> u64 {
    args.precise_timeout.unwrap_or(cfg.precise.timeout_secs)
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
    fn unknown_storage_backend_is_rejected() {
        let err = parse(
            "[storage]
backend = \"sqlite\"
",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("[storage] backend = \"sqlite\""), "{err}");
        assert!(err.contains("expected one of: bkndb"), "{err}");
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

    fn sync_args(args: &[&str]) -> SyncArgs {
        let mut argv = vec!["code-rcl", "sync"];
        argv.extend_from_slice(args);
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Sync(s) => s,
            _ => unreachable!(),
        }
    }

    #[test]
    fn resolve_sync_precedence() {
        let dflt = ProjectConfig::default();
        let s = resolve_sync(&sync_args(&[]), &dflt);
        assert_eq!(s.max_bytes, 512 * 1024);
        assert!(s.languages.is_empty() && !s.languages_from_config);

        let mut cfg = ProjectConfig::default();
        cfg.sync.max_file_kb = 1024;
        assert_eq!(resolve_sync(&sync_args(&[]), &cfg).max_bytes, 1024 * 1024);
        assert_eq!(
            resolve_sync(&sync_args(&["--max-file-kb", "8"]), &cfg).max_bytes,
            8 * 1024
        );

        cfg.sync.languages = vec!["python".to_string()];
        let s = resolve_sync(&sync_args(&[]), &cfg);
        assert_eq!(s.languages, vec!["python".to_string()]);
        assert!(s.languages_from_config);
        let s = resolve_sync(&sync_args(&["--language", "rust"]), &cfg);
        assert_eq!(s.languages, vec!["rust".to_string()]);
        assert!(!s.languages_from_config);
    }

    #[test]
    fn resolve_precise_timeout_precedence() {
        let mut cfg = ProjectConfig::default();
        let none = PreciseArgs::default();
        assert_eq!(resolve_precise_timeout(&none, &cfg), 15);
        cfg.precise.timeout_secs = 40;
        assert_eq!(resolve_precise_timeout(&none, &cfg), 40);
        let flag = PreciseArgs {
            precise_timeout: Some(30),
            ..PreciseArgs::default()
        };
        assert_eq!(resolve_precise_timeout(&flag, &cfg), 30);
    }

    #[test]
    fn apply_precise_default_is_flag_or_config() {
        let mut cfg = ProjectConfig::default();
        let mut args = PreciseArgs::default();
        cfg.apply_precise_default(&mut args);
        assert!(!args.precise);
        cfg.precise.enabled = true;
        cfg.apply_precise_default(&mut args);
        assert!(args.precise);
        cfg.precise.enabled = false;
        cfg.apply_precise_default(&mut args);
        assert!(args.precise, "config false must not turn a passed flag off");
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingsConfig {
    pub model: String,
    pub model_dir: Option<std::path::PathBuf>,
    pub dimensions: usize,
    pub batch_size: usize,
    pub threads: usize,
    pub batching: EmbeddingBatching,
    pub chunk_profile: EmbeddingChunkProfile,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingChunkProfile {
    #[default]
    Full,
    Compact,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingBatching {
    #[default]
    File,
    Length,
}
impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            model: "embeddinggemma-300m-q4".into(),
            model_dir: None,
            dimensions: 768,
            batch_size: 4,
            threads: 0,
            batching: EmbeddingBatching::File,
            chunk_profile: EmbeddingChunkProfile::Full,
        }
    }
}

impl EmbeddingsConfig {
    /// Resolve the CPU budget without changing the persisted configuration.
    pub fn effective_threads(&self) -> usize {
        self.resolve_threads(std::thread::available_parallelism().map(usize::from).ok())
    }

    fn resolve_threads(&self, available: Option<usize>) -> usize {
        if self.threads == 0 {
            available.unwrap_or(1).max(1)
        } else {
            self.threads
        }
    }
}

#[cfg(test)]
mod thread_tests {
    use super::*;

    #[test]
    fn automatic_threads_respect_available_capacity_and_manual_override() {
        let mut cfg = EmbeddingsConfig::default();
        assert_eq!(cfg.threads, 0);
        assert_eq!(cfg.resolve_threads(None), 1);
        assert_eq!(cfg.resolve_threads(Some(0)), 1);
        assert_eq!(cfg.resolve_threads(Some(12)), 12);
        assert_eq!(cfg.resolve_threads(Some(128)), 128);
        cfg.threads = 4;
        assert_eq!(cfg.resolve_threads(Some(12)), 4);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScopeConfig {
    pub categories: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Accepted for old local configurations; search uses EmbeddingGemma only.
    #[serde(rename = "ranking_model_dir")]
    pub _legacy_ranking_model_dir: Option<std::path::PathBuf>,
}
