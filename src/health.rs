//! Persist parser health by source hash and analyzer revision, including warm-cache reads.
use crate::{
    analysis::Language,
    cache::{CacheDb, models::FileRow},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::path::{Component, Path};
pub fn revision(language: Language) -> String {
    format!(
        "{}-semantic-{}",
        language.group(),
        if matches!(language, Language::C | Language::Cpp) {
            3
        } else {
            2
        }
    )
}
/// Reject old semantic generations, including explicit --no-sync consumers.
pub fn ensure_current(db: &CacheDb) -> Result<()> {
    crate::precise::state::ensure_current(db)?;
    let files = db.all_files()?;
    let context = files
        .iter()
        .any(|f| matches!(f.language.as_str(), "c" | "cpp"))
        .then(|| {
            crate::config::ProjectConfig::load(&db.project)
                .and_then(|cfg| crate::c_family::BuildContext::load_cached(&db.project, &cfg, db))
        })
        .transpose()?;
    if let Some(context) = &context {
        anyhow::ensure!(
            db.meta_get("c_family_context")?.as_deref() == Some(context.fingerprint.as_str()),
            "C/C++ build context changed; run code-rcl sync without --no-sync"
        );
    }
    for f in files {
        let stale = db
            .meta_get(&format!("semantic_stale:{}", f.path))?
            .as_deref()
            == Some("true");
        let revision_stale = if let Some(language) = context
            .as_ref()
            .and_then(|c| c.language(&db.project.join(&f.path)))
            .or_else(|| Language::from_path(Path::new(&f.path)))
        {
            db.meta_get(&format!("analyzer:{}", f.path))?
                .is_some_and(|r| {
                    r != if matches!(language, Language::C | Language::Cpp) {
                        format!(
                            "{}:{}",
                            revision(language),
                            context.as_ref().unwrap().fingerprint
                        )
                    } else {
                        revision(language)
                    }
                })
        } else {
            false
        };
        anyhow::ensure!(
            !stale && !revision_stale,
            "semantic analysis is stale for {}; run `code-rcl sync` without --no-sync",
            f.path
        );
    }
    Ok(())
}
pub fn warn(db: &CacheDb, file: &FileRow) -> Result<()> {
    if !file.parsed_ok {
        crate::operation::warning(format!(
            "parser reported errors in {} (cached diagnostics; inspect embeddings status --files)",
            file.path
        ));
        crate::operation::analysis_partial();
        crate::operation::reason("parser_error", json!({"path":file.path}));
    }
    if let Some(raw) = db.meta_get(&format!("diagnostics:{}", file.path))?
        && let Ok(diagnostics) = serde_json::from_str::<Value>(&raw)
        && diagnostics["errors"].as_array().is_some_and(|errors| {
            errors.iter().any(|e| {
                matches!(
                    e["kind"].as_str(),
                    Some("unsupported_construct" | "header_language_uncertain")
                )
            })
        })
    {
        crate::operation::analysis_partial();
        crate::operation::warning(format!(
            "{}: cached analysis has unsupported constructs or uncertain header language; inspect diagnostics and configure precise context",
            file.path
        ));
    }
    Ok(())
}
pub fn files(db: &CacheDb, project: &Path) -> Result<Value> {
    let cfg = crate::config::ProjectConfig::load(project)?;
    let key = crate::embeddings::config_key(&cfg.embeddings);
    let selected: std::collections::HashSet<_> = crate::embeddings::scope(db, &cfg)?
        .into_iter()
        .map(|f| f.path)
        .collect();
    let mut out = Vec::new();
    for f in db.all_files()? {
        let state = if !selected.contains(&f.path) {
            "excluded_configuration"
        } else if !f.parsed_ok {
            "blocked_parser"
        } else if db
            .meta_get(&format!("embedding_file:{}", f.path))?
            .as_deref()
            != Some(format!("{key}:{}", f.content_hash).as_str())
        {
            "pending_index"
        } else {
            "ready"
        };
        let diagnostics = db
            .meta_get(&format!("diagnostics:{}", f.path))?
            .map(|s| serde_json::from_str::<Value>(&s))
            .transpose()?;
        out.push(json!({"path":f.path,"source_hash":f.content_hash,"state":state,"diagnostics":diagnostics,
            "recovery":match state {"blocked_parser"=>"inspect parser diagnostics; repeated embedding sync alone cannot repair syntax/grammar errors","pending_index"=>"run embeddings sync","excluded_configuration"=>"inspect source sync language/size scope and ignore configuration",_=>"none"}}));
    }
    Ok(json!(out))
}
pub fn inspect_path(project: &Path, path: &str) -> Result<Value> {
    anyhow::ensure!(
        !Path::new(path).is_absolute()
            && !Path::new(path)
                .components()
                .any(|p| matches!(p, Component::ParentDir | Component::Prefix(_))),
        "path must be project-relative without parent traversal"
    );
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["check-ignore", "-v", "--", path])
        .output()?;
    anyhow::ensure!(
        output.status.success() || output.status.code() == Some(1),
        "git check-ignore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(
        json!({"path":path,"ignored":output.status.success(),"ignore_rule":String::from_utf8_lossy(&output.stdout).trim(),"language":Language::from_path(Path::new(path)).map(|l|l.group()),"note":"explicit path inspection; ignored trees were not scanned"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_health_survives_warm_cache_reparse_and_deletion() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("bad.kt"), "fun broken( {").unwrap();
        let mut db = CacheDb::open(d.path()).unwrap();
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        crate::service::auto_sync(&mut db, d.path()).unwrap();
        assert_eq!(files(&db, d.path()).unwrap()[0]["state"], "blocked_parser");
        let diagnostic = db.meta_get("diagnostics:bad.kt").unwrap().unwrap();
        assert!(diagnostic.contains("start_byte"));
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        let stats = crate::service::auto_sync(&mut db, d.path()).unwrap();
        assert_eq!(stats.unchanged, 1);
        assert!(!crate::operation::metadata().0.is_empty());
        assert!(crate::operation::metadata().1);
        assert!(
            !crate::operation::graph_partial(),
            "parser coverage must not imply mixed LSP resolution"
        );
        db.meta_set("analyzer:bad.kt", "old").unwrap();
        assert_eq!(
            crate::service::auto_sync(&mut db, d.path())
                .unwrap()
                .changed,
            1
        );
        std::fs::write(d.path().join("bad.kt"), "fun repaired() {}").unwrap();
        crate::service::auto_sync(&mut db, d.path()).unwrap();
        assert_eq!(files(&db, d.path()).unwrap()[0]["state"], "pending_index");
        assert_eq!(
            serde_json::from_str::<Value>(&db.meta_get("diagnostics:bad.kt").unwrap().unwrap())
                .unwrap()["parse_ok"],
            true
        );
        std::fs::remove_file(d.path().join("bad.kt")).unwrap();
        crate::service::auto_sync(&mut db, d.path()).unwrap();
        assert!(db.meta_get("diagnostics:bad.kt").unwrap().is_none());
    }
    #[test]
    fn explicit_path_explains_ignore_without_scanning_ignored_source() {
        let d = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("init")
                .arg(d.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::write(d.path().join(".gitignore"), "generated/\n").unwrap();
        assert_eq!(
            inspect_path(d.path(), "generated/api.kt").unwrap()["ignored"],
            true
        );
        assert!(inspect_path(d.path(), "../outside.rs").is_err());
    }
}
