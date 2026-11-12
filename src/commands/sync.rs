use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::analysis::{self, Language};
use crate::cache::CacheDb;
use crate::cache::models::FileRow;
use crate::cli::{PreciseArgs, SyncArgs};
use crate::config::{self, ProjectConfig};
use crate::dump::walker;
use crate::precise::{self, PreciseOptions, PreciseStats};

pub fn run(mut args: SyncArgs) -> Result<()> {
    let project = args.project.clone();
    let cfg = ProjectConfig::load(&project)?;
    cfg.apply_precise_default(&mut args.precise);
    args.precise.validate()?;
    let settings = config::resolve_sync(&args, &cfg);
    if settings.languages_from_config {
        eprintln!(
            "note: [sync] languages in .code-rcl/config.toml limits analysis to: {} (pass --language to override)",
            settings.languages.join(", ")
        );
    }
    let mut db = CacheDb::open(&project)?;
    let stats = sync_cache(&mut db, &args)?;
    let (total_symbols, total_imports) = db.totals()?;

    if !args.json {
        println!(
            "sync: {} source files (+{} ~{} ={} -{}), {} symbols ({} re-parsed), {} imports ({} re-parsed)",
            stats.scanned,
            stats.added,
            stats.changed,
            stats.unchanged,
            stats.removed,
            total_symbols,
            stats.symbols,
            total_imports,
            stats.imports,
        );
        if stats.unsupported > 0 {
            println!(
                "note: {} files recorded without analysis (analyzer not implemented yet)",
                stats.unsupported
            );
        }
    }

    if args.precise.requested() {
        let precise = run_precise(&mut db, &args)?;
        report_precise(&precise);
        // The user asked for compiler-grade resolution explicitly, so a pass
        // that broke down must not pass for success — the heuristic edges that
        // remain are not what was asked for.
        if !precise.errors.is_empty() {
            crate::operation::partial();
        }
        if !precise.errors.is_empty()
            && !args.precise.allow_heuristic_fallback
            && !args.precise.precise_selective
        {
            bail!("`--precise` did not finish for every language (see the errors above)");
        }
    }

    // The report reopens the cache to build its graph, and bkndb allows one
    // open handle per file, so release ours first.
    drop(db);

    if !args.no_report {
        // A stale or missing report must never fail a sync that itself succeeded.
        match crate::commands::report::refresh_after_sync(
            &project,
            &stats,
            args.precise.requested(),
        ) {
            Ok(Some(path)) if !args.json => println!("report: refreshed {}", path.display()),
            Ok(Some(_)) => {}
            Ok(None) => {}
            Err(e) => eprintln!("warning: could not refresh the code report: {e:#}"),
        }
    }
    if args.json {
        println!("{}", crate::output::json(&project, false, &stats)?);
    }
    Ok(())
}

/// Run the language-server pass for the languages this sync covered.
pub fn run_precise(db: &mut CacheDb, args: &SyncArgs) -> Result<PreciseStats> {
    let cfg = ProjectConfig::load(&args.project)?;
    let settings = config::resolve_sync(args, &cfg);
    let opts = precise_options(&args.precise, &settings.languages, &cfg)?;
    precise::run_precise_pass(db, &args.project, &opts)
}

pub fn precise_options(
    args: &PreciseArgs,
    language_filter: &[String],
    cfg: &ProjectConfig,
) -> Result<PreciseOptions> {
    Ok(PreciseOptions {
        selective: args.precise_selective,
        budget: Duration::from_secs(cfg.precise.selective_budget_secs),
        full: args.precise_full,
        request_timeout: Duration::from_secs(config::resolve_precise_timeout(args, cfg).max(1)),
        language_filter: language_filter.to_vec(),
        backends: precise::backend::select_backends(cfg)?,
    })
}

/// Print what the precise pass did. All output goes to stderr so stdout
/// remains clean when piped or formatting as JSON.
pub fn report_precise(stats: &PreciseStats) {
    for outcome in &stats.languages {
        eprintln!(
            "precise[{}]: {} resolved {}/{} refs in {} file(s) \
             ({} external, {} untracked, {} unresolved)",
            outcome.language,
            outcome.server,
            outcome.hits,
            outcome.queried,
            outcome.files,
            outcome.external,
            outcome.nonode,
            outcome.unresolved,
        );
    }
    if stats.languages.is_empty() && stats.warnings.is_empty() && stats.errors.is_empty() {
        eprintln!("precise: everything was already up to date (use --precise-full to redo it)");
    }
    for warning in &stats.warnings {
        eprintln!("warning: {warning}");
    }
    for error in &stats.errors {
        eprintln!("error: precise pass failed for {error}");
    }
}

#[derive(Default, serde::Serialize)]
pub struct SyncStats {
    pub scanned: usize,
    pub added: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub symbols: usize,
    pub imports: usize,
    pub unsupported: usize,
    pub content_backfilled: usize,
}

/// Incrementally bring the cache in line with the source tree. Reused by
/// `graph`'s auto-sync.
pub fn sync_cache(db: &mut CacheDb, args: &SyncArgs) -> Result<SyncStats> {
    let _time = crate::operation::time("source_sync");
    crate::precise::state::prepare(db)?;
    let project = &args.project;
    let cfg = ProjectConfig::load(project)?;
    let settings = config::resolve_sync(args, &cfg);
    let max_bytes = settings.max_bytes;
    let filter = &settings.languages;

    // Current source files on disk.
    let mut on_disk: Vec<DiskFile> = Vec::new();
    let mut out_of_scope: HashSet<String> = HashSet::new();
    let source_paths = walker::collect_source_files(project)?;
    let c_context = source_paths
        .iter()
        .any(|p| matches!(Language::from_path(p), Some(Language::C | Language::Cpp)))
        .then(|| crate::c_family::BuildContext::load_cached(project, &cfg, db))
        .transpose()?;
    for abs in source_paths {
        crate::operation::check()?;
        let Some(language) = c_context
            .as_ref()
            .and_then(|c| c.language(&abs))
            .or_else(|| Language::from_path(&abs))
        else {
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
            // Out of scope for this run, not gone: its cached rows must survive.
            let rel = abs.strip_prefix(project).unwrap_or(&abs);
            out_of_scope.insert(rel.to_string_lossy().replace('\\', "/"));
            continue;
        }
        let meta = fs::metadata(&abs)
            .with_context(|| format!("reading metadata for {}", abs.display()))?;
        if meta.len() > max_bytes {
            crate::operation::warning(format!("skipped oversized file: {}", abs.display()));
            let rel = abs.strip_prefix(project).unwrap_or(&abs);
            out_of_scope.insert(rel.to_string_lossy().replace('\\', "/"));
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

    for path in &out_of_scope {
        db.invalidate_embedding_file(path)?;
    }
    let mut generation_invalidated = false;
    // Drop files that no longer exist.
    for path in cached.keys() {
        if !disk_paths.contains(path.as_str()) && !out_of_scope.contains(path.as_str()) {
            if !generation_invalidated {
                db.invalidate_precise_generation()?;
                generation_invalidated = true;
            }
            db.delete_file(path)?;
            stats.removed += 1;
        }
    }

    crate::progress::update(|s| s["source_files_total"] = serde_json::json!(on_disk.len()));
    for (position, file) in on_disk.iter().enumerate() {
        crate::progress::phase("source_hashing");
        crate::progress::update(|s| {
            s["file"] = serde_json::json!(file.rel);
            s["source_files_done"] = serde_json::json!(position);
        });
        crate::operation::check()?;
        let existing = cached.get(&file.rel);
        let hash_timer = crate::operation::time("source_read_hash");
        let bytes =
            fs::read(&file.abs).with_context(|| format!("reading {}", file.abs.display()))?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        drop(hash_timer);
        let scope_key = format!("embedding_excluded:{}", file.rel);
        if db.meta_get(&scope_key)?.as_deref() == Some("true") {
            db.meta_set(&scope_key, "false")?;
        }

        let analyzer = if matches!(file.language, Language::C | Language::Cpp) {
            format!(
                "{}:{}",
                crate::health::revision(file.language),
                c_context.as_ref().unwrap().fingerprint
            )
        } else {
            crate::health::revision(file.language)
        };
        let analyzer_key = format!("analyzer:{}", file.rel);
        if existing.is_some_and(|f| f.content_hash == hash && f.language == file.language.group())
            && db.meta_get(&analyzer_key)?.as_deref() == Some(analyzer.as_str())
        {
            if file.language.is_supported() {
                db.ensure_source_snapshot(existing.unwrap(), &bytes)?;
                if db.ensure_content_index(existing.unwrap(), &bytes)? {
                    stats.content_backfilled += 1;
                }
            }
            db.update_file_metadata(existing.unwrap().id, file.mtime, Some(file.size))?;
            stats.unchanged += 1;
            crate::health::warn(db, existing.unwrap())?;
            crate::progress::completed();
            continue;
        }
        let is_new = existing.is_none();

        if !generation_invalidated {
            db.invalidate_precise_generation()?;
            generation_invalidated = true;
        }

        let source = String::from_utf8_lossy(&bytes);
        crate::progress::phase("parsing");
        let parse_timer = crate::operation::time("parsing");
        let mut parsed = analysis::parse_file(file.language, &source);
        analysis::qualify_file(&mut parsed, &file.rel, file.language);
        if matches!(file.language, Language::C | Language::Cpp)
            && let Some(context) = &c_context
        {
            let header = context.headers.get(&crate::c_family::absolute(&file.abs));
            if header.is_some_and(|h| !h.clear) {
                let header = header.unwrap();
                parsed.diagnostics.push(serde_json::json!({"kind":"header_language_uncertain","language":file.language.group(),"source":header.language_source,"message":"language is inferred; configure c_family.header_language_overrides before precise"}));
                crate::operation::analysis_partial();
                crate::operation::warning(format!(
                    "{}: header language is uncertain; configure c_family.header_language_overrides before precise",
                    file.rel
                ));
            }
            for symbol in &mut parsed.symbols {
                symbol.semantic.language_details.insert("build_context".into(), serde_json::json!({"fingerprint":context.fingerprint,"header_language_clear":header.map(|h|h.clear),"header_language_source":header.map(|h|h.language_source),"includer":header.and_then(|h|h.includer.as_ref()).map(|p|p.to_string_lossy())}));
            }
        }
        drop(parse_timer);
        if parsed
            .diagnostics
            .iter()
            .any(|d| d["kind"] == "unsupported_construct")
        {
            crate::operation::analysis_partial();
            crate::operation::warning(format!(
                "{}: source includes namespace alias/using constructs requiring precise lookup",
                file.rel
            ));
        }
        stats.symbols += parsed.symbols.len();
        stats.imports += parsed.imports.len();
        if !parsed.parse_ok {
            crate::operation::warning(format!("parser reported errors in {}", file.rel));
            crate::operation::analysis_partial();
            crate::operation::reason("parser_error", serde_json::json!({"path":file.rel}));
        }
        if !file.language.is_supported() {
            stats.unsupported += 1;
        }

        crate::progress::phase("source_commit");
        let commit_timer = crate::operation::time("source_commit");
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
            &parsed.scopes,
            &parsed.bindings,
            &parsed.string_literals,
            file.language.is_supported().then_some(bytes.as_slice()),
        )
        .with_context(|| format!("caching analysis for {}", file.rel))?;

        drop(commit_timer);
        crate::progress::completed();
        db.meta_set(&analyzer_key, &analyzer)?;
        db.meta_set(&format!("diagnostics:{}",file.rel), &serde_json::json!({"source_hash":hash,"analyzer":analyzer,"parse_ok":parsed.parse_ok,"errors":parsed.diagnostics}).to_string())?;

        if is_new {
            stats.added += 1;
        } else {
            stats.changed += 1;
        }
    }

    crate::progress::update(|s| s["source_files_done"] = s["source_files_total"].clone());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    crate::operation::check()?;
    db.meta_set("last_sync", &now.to_string())?;
    if let Some(context) = &c_context {
        db.meta_set("c_family_context", &context.fingerprint)?;
    }
    if stats.content_backfilled > 0 {
        crate::operation::reason(
            "content_index_updated",
            serde_json::json!({
                "version": crate::cache::content::VERSION,
                "backfilled_files": stats.content_backfilled,
                "embedding_rebuild_required": false,
            }),
        );
    }

    Ok(stats)
}

struct DiskFile {
    rel: String,
    language: Language,
    abs: std::path::PathBuf,
    size: i64,
    mtime: Option<i64>,
}
