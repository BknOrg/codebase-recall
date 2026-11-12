//! Cache policy, backend identity and observable verification coverage.
use super::{
    backend::{Backend, Launcher},
    policy,
};
use crate::{cache::CacheDb, config::ProjectConfig};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

pub fn file_digest(path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path)?;
    let key = format!(
        "precision_file:{}:{}:{:?}",
        path.display(),
        metadata.len(),
        metadata.modified()?
    );
    if let Some(value) = crate::operation::memo(&key) {
        return Ok(value);
    }
    let mut file = std::fs::File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    loop {
        let size = std::io::Read::read(&mut file, &mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    let value = hash.finalize().to_hex().to_string();
    crate::operation::memo_set(key, value.clone());
    Ok(value)
}
// Server binaries can be hundreds of MB. Reuse their content identity while
// filesystem identity is unchanged; project sources still use conservative hashes.
fn executable_digest(db: &CacheDb, path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path)?;
    let stamp = format!(
        "{}:{:?}:{:?}",
        metadata.len(),
        metadata.modified()?,
        metadata.created().ok()
    );
    let key = format!(
        "precision_executable:{}",
        blake3::hash(path.to_string_lossy().as_bytes()).to_hex()
    );
    if let Some(raw) = db.meta_get(&key)?
        && let Ok(value) = serde_json::from_str::<Value>(&raw)
        && value["stamp"].as_str() == Some(stamp.as_str())
        && let Some(hash) = value["hash"].as_str()
    {
        return Ok(hash.into());
    }
    let hash = file_digest(path)?;
    db.meta_set(&key, &json!({"stamp":stamp,"hash":hash}).to_string())?;
    Ok(hash)
}
fn package_identity(
    hash: &mut blake3::Hasher,
    db: &CacheDb,
    backend: &Backend,
    launcher: &Launcher,
) -> Result<()> {
    let packages: &[&str] = match backend.lang_group {
        "javascript" | "typescript" => &["typescript-language-server", "typescript"],
        "vue" => &[
            "@vue/language-server",
            "@vue/typescript-plugin",
            "typescript",
        ],
        "svelte" => &["svelte-language-server", "svelte", "typescript"],
        "python" => &["pyright"],
        _ => &[],
    };
    let mut roots: Vec<_> = db
        .project
        .ancestors()
        .map(|p| p.join("node_modules"))
        .collect();
    if let Some(parent) = launcher.program.parent() {
        roots.push(parent.join("node_modules"));
        roots.push(parent.to_owned());
        if let Some(parent) = parent.parent() {
            roots.push(parent.to_owned());
        }
    }
    roots.sort();
    roots.dedup();
    for root in roots {
        for package in packages {
            let dir = root.join(package);
            let manifest = dir.join("package.json");
            if !manifest.is_file() {
                continue;
            }
            hash.update(manifest.to_string_lossy().as_bytes());
            hash.update(file_digest(&manifest)?.as_bytes());
            if let Ok(value) = serde_json::from_slice::<Value>(&std::fs::read(&manifest)?) {
                let mut paths = Vec::new();
                if let Some(main) = value["main"].as_str() {
                    paths.push(main);
                }
                if let Some(bin) = value["bin"].as_str() {
                    paths.push(bin);
                } else if let Some(bins) = value["bin"].as_object() {
                    paths.extend(bins.values().filter_map(Value::as_str));
                }
                for path in paths {
                    let path = dir.join(path);
                    if path.is_file() {
                        hash.update(executable_digest(db, &path)?.as_bytes());
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn fingerprint(
    db: &CacheDb,
    backend: &Backend,
    launcher: &Launcher,
    cfg: &ProjectConfig,
) -> Result<String> {
    let generation = db.meta_get("generation")?.unwrap_or_default();
    let base_key = format!("precision_project:{}:{generation}", db.project.display());
    let base = if let Some(value) = crate::operation::memo(&base_key) {
        value
    } else {
        let mut hash = blake3::Hasher::new();
        let mut files = crate::dump::walker::collect_source_files(&db.project)?;
        files.sort();
        for file in files {
            crate::operation::check()?;
            let name = file.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if matches!(
                name,
                "Cargo.toml"
                    | "Cargo.lock"
                    | "go.mod"
                    | "go.sum"
                    | "go.work"
                    | "package.json"
                    | "package-lock.json"
                    | "pnpm-lock.yaml"
                    | "yarn.lock"
                    | "pyproject.toml"
                    | "pyrightconfig.json"
                    | "requirements.txt"
                    | "pom.xml"
                    | "build.gradle"
                    | "build.gradle.kts"
                    | "settings.gradle"
                    | "settings.gradle.kts"
                    | "gradle.properties"
                    | "gradle.lockfile"
                    | ".classpath"
                    | ".clangd"
            ) || name.starts_with("tsconfig") && name.ends_with(".json")
                || name.starts_with("jsconfig") && name.ends_with(".json")
                || name.starts_with("svelte.config.")
                || name.starts_with("vue.config.")
            {
                hash.update(file.to_string_lossy().as_bytes());
                hash.update(file_digest(&file)?.as_bytes());
            }
        }
        let value = hash.finalize().to_hex().to_string();
        crate::operation::memo_set(base_key, value.clone());
        value
    };
    let mut hash = blake3::Hasher::new();
    hash.update(policy::VERSION.as_bytes());
    hash.update(generation.as_bytes());
    hash.update(base.as_bytes());
    hash.update(launcher.program.to_string_lossy().as_bytes());
    hash.update(executable_digest(db, &launcher.program)?.as_bytes());
    package_identity(&mut hash, db, backend, launcher)?;
    hash.update(serde_json::to_string(&launcher.args)?.as_bytes());
    hash.update(
        format!(
            "{:?}:{:?}:{:?}",
            cfg.precise.kotlin, cfg.precise.vue, cfg.precise.clangd
        )
        .as_bytes(),
    );
    if matches!(backend.lang_group, "c" | "cpp") {
        hash.update(
            crate::c_family::BuildContext::load_cached(&db.project, cfg, db)?
                .fingerprint
                .as_bytes(),
        );
    }
    if backend.lang_group == "vue" {
        let dependencies = super::vue::dependencies(&db.project, launcher, cfg)?;
        for file in [
            dependencies.sdk.join("typescript.js"),
            dependencies.plugin.join("package.json"),
        ] {
            hash.update(file.to_string_lossy().as_bytes());
            hash.update(file_digest(&file)?.as_bytes());
        }
        let ts = super::backend::BACKENDS
            .iter()
            .find(|b| b.lang_group == "typescript")
            .unwrap()
            .locate()
            .map_err(anyhow::Error::msg)?;
        hash.update(ts.program.to_string_lossy().as_bytes());
        hash.update(executable_digest(db, &ts.program)?.as_bytes());
    }
    Ok(hash.finalize().to_hex().to_string())
}
pub fn prepare(db: &mut CacheDb) -> Result<()> {
    if db.meta_get("precise_policy_version")?.as_deref() != Some(policy::VERSION) {
        if db.all_refs()?.iter().any(|r| r.precise_status.is_some()) {
            db.invalidate_precise_generation()?;
        }
        db.meta_set("precise_policy_version", policy::VERSION)?;
        db.meta_set("precision_summary", "")?;
    }
    let files = db.all_files()?;
    if db.meta_get("precision_has_results")?.as_deref() != Some("true")
        && !files.iter().any(|f| f.precise_synced_at.is_some())
    {
        return Ok(());
    }
    let cfg = ProjectConfig::load(&db.project)?;
    let references = db.all_refs()?;
    let mut invalidated = false;
    for backend in super::backend::select_backends(&cfg)? {
        let saved = db
            .meta_get(&format!("precision_backend:{}", backend.lang_group))?
            .filter(|v| !v.is_empty());
        let ids: HashSet<_> = files
            .iter()
            .filter(|f| f.language == backend.lang_group)
            .map(|f| f.id)
            .collect();
        if !references
            .iter()
            .any(|r| ids.contains(&r.file_id) && r.precise_status.is_some())
        {
            continue;
        }
        let current = backend
            .locate()
            .ok()
            .and_then(|l| fingerprint(db, backend, &l, &cfg).ok());
        if saved.is_none() || current != saved {
            invalidated = true;
            db.clear_precise(&ids.into_iter().collect::<Vec<_>>())?;
            db.meta_set("precise_generation", "invalid")?;
            db.meta_set(&format!("precision_backend:{}", backend.lang_group), "")?;
            db.meta_set("precision_summary", "")?;
        }
    }
    if invalidated {
        db.meta_set(
            "precision_has_results",
            if db.all_refs()?.iter().any(|r| r.precise_status.is_some()) {
                "true"
            } else {
                "false"
            },
        )?;
    }
    Ok(())
}
pub fn ensure_current(db: &CacheDb) -> Result<()> {
    let files = db.all_files()?;
    if db.meta_get("precision_has_results")?.as_deref() != Some("true")
        && !files.iter().any(|f| f.precise_synced_at.is_some())
    {
        return Ok(());
    }
    let refs = db.all_refs()?;
    if !refs.iter().any(|r| r.precise_status.is_some()) {
        return Ok(());
    }
    anyhow::ensure!(
        db.meta_get("precise_policy_version")?.as_deref() == Some(policy::VERSION),
        "precise cache policy is stale; run code-rcl sync without --no-sync"
    );
    let cfg = ProjectConfig::load(&db.project)?;
    for backend in super::backend::select_backends(&cfg)? {
        let ids: HashSet<_> = files
            .iter()
            .filter(|f| f.language == backend.lang_group)
            .map(|f| f.id)
            .collect();
        if !refs
            .iter()
            .any(|r| ids.contains(&r.file_id) && r.precise_status.is_some())
        {
            continue;
        }
        let current = backend
            .locate()
            .ok()
            .and_then(|l| fingerprint(db, backend, &l, &cfg).ok());
        anyhow::ensure!(
            current.is_some()
                && current == db.meta_get(&format!("precision_backend:{}", backend.lang_group))?,
            "{} precise backend or project context changed; run code-rcl sync without --no-sync",
            backend.lang_group
        );
    }
    Ok(())
}
pub fn metadata(db: &CacheDb) -> Result<Value> {
    let generation = db.meta_get("generation")?.unwrap_or_default();
    let saved = db
        .meta_get("precision_summary")?
        .and_then(|v| serde_json::from_str::<Value>(&v).ok());
    Ok(saved.filter(|v|v["generation"].as_str()==Some(generation.as_str())).unwrap_or_else(||json!({"requested_mode":"heuristic","policy":policy::VERSION,"languages":[],"complete":false})))
}
