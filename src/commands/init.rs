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

    let db = CacheDb::open(project)?;
    db.meta_set("root", &project.display().to_string())?;
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

    ensure_gitignored(project)?;

    println!(
        "Initialized code-rcl cache at {}",
        cache::ctx_dir(project).display()
    );
    println!("Next: `code-rcl sync` to populate the graph cache.");
    Ok(())
}

/// Make sure the project's `.gitignore` ignores `.code-ctx/`.
fn ensure_gitignored(project: &Path) -> Result<()> {
    let gitignore = project.join(".gitignore");
    let entry = format!("{}/", cache::CODE_CTX_DIR);

    let already = fs::read_to_string(&gitignore)
        .map(|c| {
            c.lines()
                .any(|l| l.trim() == entry || l.trim() == cache::CODE_CTX_DIR)
        })
        .unwrap_or(false);
    if already {
        return Ok(());
    }

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore)
        .with_context(|| format!("opening {}", gitignore.display()))?;
    writeln!(f, "{entry}")?;
    Ok(())
}
