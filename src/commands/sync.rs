use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::analysis::{self, Language};
use crate::cache::models::FileRow;
use crate::cache::CacheDb;
use crate::cli::SyncArgs;
use crate::walker;

pub fn run(args: SyncArgs) -> Result<()> {
    let project = args.project.clone();
    let mut db = CacheDb::open(&project)?;
    let stats = sync_cache(&mut db, &args)?;

    println!(
        "sync: {} source files (+{} ~{} ={} -{}), {} symbols, {} imports",
        stats.scanned,
        stats.added,
        stats.changed,
        stats.unchanged,
        stats.removed,
        stats.symbols,
        stats.imports,
    );
    if stats.unsupported > 0 {
        println!(
            "note: {} files recorded without analysis (analyzer not implemented yet)",
            stats.unsupported
        );
    }
    Ok(())
}

#[derive(Default)]
pub struct SyncStats {
    pub scanned: usize,
    pub added: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub symbols: usize,
    pub imports: usize,
    pub unsupported: usize,
}

/// Incrementally bring the cache in line with the source tree. Reused by
/// `graph`'s auto-sync.
pub fn sync_cache(db: &mut CacheDb, args: &SyncArgs) -> Result<SyncStats> {
    let project = &args.project;
    let max_bytes = args.max_file_kb.saturating_mul(1024);
    let filter = &args.language;

    // Current source files on disk.
    let mut on_disk: Vec<DiskFile> = Vec::new();
    for abs in walker::collect_source_files(project)? {
        let Some(language) = Language::from_path(&abs) else {
            continue;
        };
        // Skip minified / vendored bundles (e.g. `d3.min.js`). They aren't
        // project source and a single one can dump thousands of bogus symbols.
        if abs
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".min."))
        {
            continue;
        }
        if !filter.is_empty() && !filter.iter().any(|t| language.matches_filter(t)) {
            continue;
        }
        let Ok(meta) = fs::metadata(&abs) else {
            continue;
        };
        if meta.len() > max_bytes {
            continue;
        }
        let rel = abs.strip_prefix(project).unwrap_or(&abs);
        let rel = rel.to_string_lossy().replace('\\', "/");
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        on_disk.push(DiskFile {
            rel,
            language,
            abs,
            size: meta.len() as i64,
            mtime,
        });
    }

    let cached: HashMap<String, FileRow> = db
        .all_files()?
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    let disk_paths: HashSet<&str> = on_disk.iter().map(|d| d.rel.as_str()).collect();

    let mut stats = SyncStats {
        scanned: on_disk.len(),
        ..SyncStats::default()
    };

    // Drop files that no longer exist.
    for path in cached.keys() {
        if !disk_paths.contains(path.as_str()) {
            db.delete_file(path)?;
            stats.removed += 1;
        }
    }

    for file in &on_disk {
        let Ok(bytes) = fs::read(&file.abs) else {
            continue;
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();

        let existing = cached.get(&file.rel);
        if existing.is_some_and(|f| f.content_hash == hash) {
            stats.unchanged += 1;
            continue;
        }
        let is_new = existing.is_none();

        let source = String::from_utf8_lossy(&bytes);
        let parsed = analysis::parse_file(file.language, &source);
        stats.symbols += parsed.symbols.len();
        stats.imports += parsed.imports.len();
        if !file.language.is_supported() {
            stats.unsupported += 1;
        }

        db.replace_file_analysis(
            &file.rel,
            file.language.group(),
            &hash,
            file.mtime,
            Some(file.size),
            parsed.parse_ok,
            &parsed.symbols,
            &parsed.imports,
            &parsed.refs,
        )
        .with_context(|| format!("caching analysis for {}", file.rel))?;

        if is_new {
            stats.added += 1;
        } else {
            stats.changed += 1;
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    db.meta_set("last_sync", &now.to_string())?;

    Ok(stats)
}

struct DiskFile {
    rel: String,
    language: Language,
    abs: std::path::PathBuf,
    size: i64,
    mtime: Option<i64>,
}
