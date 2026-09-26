use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::cache::{self, CacheDb};
use crate::cli::InitArgs;

use crate::config::CONFIG_FILE;

const DEFAULT_CONFIG: &str = r#"# code-rcl project configuration
# Precedence: command-line flag > this file > built-in default.
# Every key is optional; a missing key uses the built-in default shown here.
schema_version = 1

[storage]
# Accepted but not applied yet.
backend = "bkndb"

[sync]
# Skip source files larger than this many KB.
max_file_kb = 512
# Restrict analysis to these languages. Omit (or leave empty) to analyze every
# supported language: rust, javascript, typescript, python, java, kotlin, vue,
# svelte, go, toml.
# languages = ["rust", "python"]

[graph]
# Drop resolved edges below this confidence (0.0 to 1.0).
min_confidence = 0.4
# Include edges to external (npm / pypi / crate) modules.
include_external = false
# Cap on total graph nodes (0 disables the cap).
max_nodes = 4000
# BFS depth around --focus.
depth = 2
# Edge kinds to include.
kinds = ["imports", "calls", "contains", "implements"]

[precise]
# true acts like passing --precise for sync, graph and serve.
enabled = false
# Seconds to wait for a single language-server answer.
timeout_secs = 15

[precise.kotlin]
# Kotlin language server: "jetbrains" (kotlin-lsp, default),
# "fwcd" (kotlin-language-server) or "auto" (jetbrains, falling back to fwcd).
server = "jetbrains"
"#;

pub fn run(args: InitArgs) -> Result<()> {
    let project = &args.project;
    let project_normalized = project.display().to_string().replace('\\', "/");

    let db = CacheDb::open(project)?;
    db.meta_set("root", &project_normalized)?;
    db.meta_set("schema_version", &cache::schema::SCHEMA_VERSION.to_string())?;
    if db.meta_get("created_at")?.is_none() {
        db.meta_set(
            "created_at",
            &std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .to_string(),
        )?;
    }

    let config_path = cache::ctx_dir(project).join(CONFIG_FILE);
    if !config_path.exists() || args.force {
        fs::write(&config_path, DEFAULT_CONFIG)
            .with_context(|| format!("writing {}", config_path.display()))?;
    }

    ensure_git_excluded(project)?;

    let ctx_display = cache::ctx_dir(project)
        .display()
        .to_string()
        .replace('\\', "/");
    println!("Initialized code-rcl cache at {}", ctx_display);
    println!("Next: `code-rcl sync` to populate the graph cache.");
    Ok(())
}

/// Make sure the project's `.git/info/exclude` ignores `.code-rcl/`.
fn ensure_git_excluded(project: &Path) -> Result<()> {
    let git_dir = project.join(".git");
    if !git_dir.exists() {
        return Ok(());
    }

    let exclude_dir = git_dir.join("info");
    if !exclude_dir.exists() {
        fs::create_dir_all(&exclude_dir)
            .with_context(|| format!("creating directory {}", exclude_dir.display()))?;
    }

    let exclude_file = exclude_dir.join("exclude");
    let entry = format!("{}/", cache::CODE_CTX_DIR);

    let content = fs::read_to_string(&exclude_file).unwrap_or_default();
    let already = content
        .lines()
        .any(|l| l.trim() == entry || l.trim() == cache::CODE_CTX_DIR);
    if already {
        return Ok(());
    }

    let mut prefix = "";
    if !content.is_empty() && !content.ends_with('\n') {
        prefix = "\n";
    }

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&exclude_file)
        .with_context(|| format!("opening {}", exclude_file.display()))?;
    write!(f, "{prefix}{entry}\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use std::path::Path;

    #[test]
    fn default_config_parses_to_builtin_defaults() {
        let cfg = ProjectConfig::from_toml_str(DEFAULT_CONFIG, Path::new("config.toml")).unwrap();
        let dflt = ProjectConfig::default();
        assert_eq!(cfg.graph, dflt.graph);
        assert_eq!(cfg.precise, dflt.precise);
        assert_eq!(cfg.sync.max_file_kb, dflt.sync.max_file_kb);
        assert_eq!(cfg.sync.languages, dflt.sync.languages);
    }
}
