use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::cache::{self, CacheDb};
use crate::cli::InitArgs;

const CONFIG_FILE: &str = "config.toml";

const DEFAULT_CONFIG: &str = r#"# code-rcl project configuration
schema_version = 1

[sync]
# Skip source files larger than this many KB.
max_file_kb = 512
# Languages to analyze.
languages = ["rust", "javascript", "typescript", "python", "java", "kotlin"]

[graph]
# Drop resolved edges below this confidence.
min_confidence = 0.4
# Include edges to external (npm / pypi / crate) modules.
include_external = false
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
