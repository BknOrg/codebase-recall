//! Bounded verification scheduler. A failed backend never erases another backend's answers.
use super::{
    PreciseOptions, PreciseStats, Resolver,
    backend::{Backend, Launcher},
    policy::{Coverage, Decision},
    state,
};
use crate::{
    cache::{
        CacheDb,
        models::{FileRow, RefRow},
    },
    config::ProjectConfig,
};
use anyhow::Result;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
struct Job {
    backend: &'static Backend,
    launcher: Launcher,
    languages: Vec<String>,
    decisions: Vec<Decision>,
    fingerprints: Vec<(String, String)>,
}
pub fn run(db: &mut CacheDb, project: &Path, opts: &PreciseOptions) -> Result<PreciseStats> {
    let _timer = crate::operation::time("precise_resolution");
    state::prepare(db)?;
    let files = db.all_files()?;
    let symbols = db.all_symbols()?;
    let references = db.all_refs()?;
    let cfg = ProjectConfig::load(project)?;
    let file_by_id: HashMap<_, _> = files.iter().map(|f| (f.id, f)).collect();
    let ref_by_id: HashMap<_, _> = references.iter().map(|r| (r.id, r)).collect();
    let decisions = if opts.selective {
        super::policy::decisions(db, &files, &symbols, &references)?
    } else {
        references
            .iter()
            .filter_map(|r| {
                file_by_id.get(&r.file_id).map(|f| Decision {
                    reference: r.id,
                    file: r.file_id,
                    language: f.language.clone(),
                    path: f.path.clone(),
                    offset: r.name_start_byte,
                    selected: true,
                    priority: 3,
                    reasons: vec!["full".into()],
                })
            })
            .collect()
    };
    let selected: Vec<_> = decisions
        .into_iter()
        .filter(|d| super::language_selected(&d.language, &opts.language_filter))
        .collect();
    let mut coverage: BTreeMap<String, Coverage> = BTreeMap::new();
    let mut pending: BTreeMap<String, Vec<Decision>> = BTreeMap::new();
    let mut stats = PreciseStats::default();
    let mut records = Records {
        generation: db.meta_get("generation")?.unwrap_or_default(),
        values: HashMap::new(),
    };
    for decision in &selected {
        let c = coverage
            .entry(decision.language.clone())
            .or_insert_with(|| Coverage {
                language: decision.language.clone(),
                ..Default::default()
            });
        for reason in &decision.reasons {
            *c.selection_reasons.entry(reason.clone()).or_default() += 1;
        }
        if !decision.selected {
            c.skipped += 1;
            record(&mut records, decision, "skipped", None)?;
            continue;
        }
        c.candidates += 1;
        let reference = ref_by_id[&decision.reference];
        if !opts.full && reference.precise_status.is_some() {
            c.cache_hits += 1;
            count_status(c, reference);
            record(&mut records, decision, "cached", None)?;
            continue;
        }
        if decision.offset.is_none() {
            c.pending += 1;
            c.reasons.push("unsupported_position".into());
            record(
                &mut records,
                decision,
                "unqueryable",
                Some("parser did not provide a source position"),
            )?;
            continue;
        }
        record(&mut records, decision, "pending", None)?;
        pending
            .entry(decision.language.clone())
            .or_default()
            .push(decision.clone());
    }
    let deadline = opts.selective.then(|| Instant::now() + opts.budget);
    let backends = &opts.backends;
    let mut jobs: Vec<Job> = Vec::new();
    for (language, decisions) in &pending {
        crate::operation::check()?;
        let Some(&backend) = backends.iter().find(|b| b.lang_group == language) else {
            failure(
                &mut records,
                decisions,
                "unsupported_backend",
                &format!(
                    "no precise backend; supported: {}",
                    super::backend::supported_languages()
                ),
                &mut coverage,
                &mut stats,
            )?;
            continue;
        };
        if expired(deadline) {
            failure(
                &mut records,
                decisions,
                "budget_exhausted",
                "selective time budget exhausted before server discovery",
                &mut coverage,
                &mut stats,
            )?;
            continue;
        }
        coverage.get_mut(language).unwrap().server = Some(backend.env_override.into());
        let launcher = match backend.locate() {
            Ok(l) => l,
            Err(why) => {
                failure(
                    &mut records,
                    decisions,
                    "server_missing",
                    &why,
                    &mut coverage,
                    &mut stats,
                )?;
                continue;
            }
        };
        let fingerprint = match state::fingerprint(db, backend, &launcher, &cfg) {
            Ok(f) => f,
            Err(e) => {
                failure(
                    &mut records,
                    decisions,
                    "context_unready",
                    &format!("{e:#}"),
                    &mut coverage,
                    &mut stats,
                )?;
                continue;
            }
        };
        coverage.get_mut(language).unwrap().server = Some(launcher.program.display().to_string());
        let cooldown_key = format!("precision_failure:{language}");
        if opts.selective
            && !opts.full
            && let Some(raw) = db.meta_get(&cooldown_key)?
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
            && value["fingerprint"].as_str() == Some(&fingerprint)
            && value["until"].as_u64().is_some_and(|until| until > now())
        {
            failure(
                &mut records,
                decisions,
                "cooldown",
                "previous attempt failed; retry in 60 seconds or use --precise-full",
                &mut coverage,
                &mut stats,
            )?;
            continue;
        }
        let family = family(language);
        if let Some(job) = jobs.iter_mut().find(|j| {
            family == self::family(j.backend.lang_group)
                && j.launcher.program == launcher.program
                && j.launcher.args == launcher.args
        }) {
            job.languages.push(language.clone());
            job.decisions.extend(decisions.clone());
            job.fingerprints.push((language.clone(), fingerprint));
        } else {
            jobs.push(Job {
                backend,
                launcher,
                languages: vec![language.clone()],
                decisions: decisions.clone(),
                fingerprints: vec![(language.clone(), fingerprint)],
            });
        }
    }
    jobs.sort_by(|a, b| {
        a.decisions
            .iter()
            .map(|d| d.priority)
            .min()
            .cmp(&b.decisions.iter().map(|d| d.priority).min())
            .then(b.decisions.len().cmp(&a.decisions.len()))
            .then(a.backend.lang_group.cmp(b.backend.lang_group))
    });
    let mut symbols_by_file: HashMap<_, Vec<_>> = HashMap::new();
    for s in &symbols {
        symbols_by_file.entry(s.file_id).or_default().push(s);
    }
    let file_id_by_path = files.iter().map(|f| (f.path.clone(), f.id)).collect();
    let canonical_root = std::fs::canonicalize(project)?;
    let server_root = super::map::strip_extended_prefix(&canonical_root);
    for mut job in jobs {
        crate::operation::check()?;
        if expired(deadline) {
            failure(
                &mut records,
                &job.decisions,
                "budget_exhausted",
                "selective operation deadline exhausted",
                &mut coverage,
                &mut stats,
            )?;
            continue;
        }
        job.decisions.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then(a.path.cmp(&b.path))
                .then(a.offset.cmp(&b.offset))
        });
        let ids: HashSet<_> = job.decisions.iter().map(|d| d.file).collect();
        let mut pending_files: Vec<&FileRow> =
            files.iter().filter(|f| ids.contains(&f.id)).collect();
        pending_files.sort_by_key(|f| {
            job.decisions
                .iter()
                .position(|d| d.file == f.id)
                .unwrap_or(usize::MAX)
        });
        let preflight = (|| -> Result<()> {
            if job.backend.lang_group == "vue" {
                let deps = super::vue::dependencies(project, &job.launcher, &cfg)?;
                job.launcher
                    .args
                    .push(format!("--tsdk={}", deps.sdk.display()));
            }
            if matches!(job.backend.lang_group, "c" | "cpp") {
                let context = crate::c_family::BuildContext::load_cached(project, &cfg, db)?;
                context.validate_precise(project, &pending_files)?;
                job.launcher.args.push(format!(
                    "--compile-commands-dir={}",
                    context.directory.unwrap().display()
                ));
                if !cfg.precise.clangd.query_drivers.is_empty() {
                    job.launcher.args.push(format!(
                        "--query-driver={}",
                        cfg.precise.clangd.query_drivers.join(",")
                    ));
                }
            } else if job.backend.lacks_project_model(project)
                && matches!(job.backend.lang_group, "rust" | "java" | "kotlin" | "go")
            {
                anyhow::bail!(
                    "missing {}: {}",
                    job.backend.project_markers.join(" / "),
                    job.backend.marker_hint
                );
            }
            Ok(())
        })();
        if let Err(error) = preflight {
            failure(
                &mut records,
                &job.decisions,
                "context_unready",
                &format!("{error:#}"),
                &mut coverage,
                &mut stats,
            )?;
            continue;
        }
        let chosen: HashMap<_, _> = job
            .decisions
            .iter()
            .enumerate()
            .map(|(i, d)| (d.reference, i))
            .collect();
        if opts.full {
            db.clear_precise_references(&chosen.keys().copied().collect::<Vec<_>>())?;
            db.meta_set("precise_generation", "invalid")?;
        }
        let starts = super::client::server_starts();
        let mut resolver = Resolver {
            db,
            project: &server_root,
            canonical_root: &canonical_root,
            symbols_by_file: &symbols_by_file,
            file_id_by_path: &file_id_by_path,
            uri_cache: HashMap::new(),
            opts,
            selected: opts.selective.then_some(&chosen),
            deadline,
            cfg: &cfg,
        };
        let outcome = resolver.run_language(job.backend, &job.launcher, &pending_files);
        coverage
            .get_mut(job.backend.lang_group)
            .unwrap()
            .server_starts += super::client::server_starts() - starts;
        crate::operation::check()?;
        let current_refs: HashMap<_, _> = db.all_refs()?.into_iter().map(|r| (r.id, r)).collect();
        let failure_reason = match &outcome {
            Err(e) => Some(format!("{e:#}")),
            _ => None,
        };
        let mut incomplete = false;
        for decision in &job.decisions {
            let c = coverage.get_mut(&decision.language).unwrap();
            if let Some(r) = current_refs.get(&decision.reference)
                && r.precise_status.is_some()
            {
                c.queried += 1;
                count_status(c, r);
                record(&mut records, decision, "checked", None)?;
            } else {
                let saved = db
                    .meta_get(&format!("precision_ref:{}", decision.reference))?
                    .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
                if saved.as_ref().is_some_and(|v| v["attempted"] == true) {
                    c.queried += 1;
                }
                incomplete = true;
                let request_reason = saved
                    .as_ref()
                    .and_then(|v| v["error"].as_str())
                    .or(failure_reason.as_deref());
                let code = if expired(deadline) {
                    "budget_exhausted"
                } else {
                    error_code(request_reason.unwrap_or("request_failed"))
                };
                c.pending += 1;
                c.failed += usize::from(code != "budget_exhausted");
                c.reasons.push(code.into());
                record(
                    &mut records,
                    decision,
                    if code == "budget_exhausted" {
                        "pending"
                    } else {
                        "failed"
                    },
                    request_reason,
                )?;
            }
        }
        match outcome {
            Ok((outcome, warnings)) => {
                stats.languages.push(outcome);
                stats.warnings.extend(warnings);
            }
            Err(e) => stats.errors.push(format!(
                "{}: {e:#}; {}",
                job.languages.join(","),
                job.backend.install_hint
            )),
        }
        if incomplete {
            let message = format!(
                "{}: {} references remain unverified{}",
                job.languages.join(","),
                job.decisions
                    .iter()
                    .filter(|d| current_refs
                        .get(&d.reference)
                        .is_none_or(|r| r.precise_status.is_none()))
                    .count(),
                if expired(deadline) {
                    " (budget_exhausted)"
                } else {
                    " (request_failed)"
                }
            );
            stats.warnings.push(message);
            for (language, fingerprint) in &job.fingerprints {
                db.meta_set(
                    &format!("precision_failure:{language}"),
                    &json!({"fingerprint":fingerprint,"until":now()+60}).to_string(),
                )?;
            }
        }
        for (language, fingerprint) in &job.fingerprints {
            db.meta_set(&format!("precision_backend:{language}"), fingerprint)?;
        }
    }
    db.meta_set_many(&records.values.into_iter().collect::<Vec<_>>())?;
    let current = db.all_refs()?;
    let has_results = current.iter().any(|r| r.precise_status.is_some());
    db.meta_set(
        "precision_has_results",
        if has_results { "true" } else { "false" },
    )?;
    let pending_count: usize = coverage.values().map(|c| c.pending).sum();
    if pending_count > 0 && stats.errors.is_empty() && stats.warnings.is_empty() {
        stats.warnings.push(format!("unsupported_position: {pending_count} references cannot be queried; parser/source mapping needs attention"));
    }
    if !opts.selective && stats.errors.is_empty() && pending_count == 0 {
        // Completion covers all source languages, rather than just this invocation's filter.
        let all_complete = files
            .iter()
            .filter(|f| !crate::analysis::lang::is_config_group(&f.language))
            .all(|f| {
                current
                    .iter()
                    .filter(|r| r.file_id == f.id && r.name_start_byte.is_some())
                    .all(|r| r.precise_status.is_some())
            });
        if all_complete {
            for file in &files {
                if file.precise_synced_at.is_none() {
                    db.set_precise_subset(file.id, &[], true, &[])?;
                }
            }
            db.meta_set(
                "precise_generation",
                &db.meta_get("generation")?.unwrap_or_else(|| "0".into()),
            )?;
        }
    }
    let complete = pending_count == 0 && stats.errors.is_empty();
    for language in coverage.values_mut() {
        language.reasons.sort();
        language.reasons.dedup();
    }
    let summary = json!({"generation":db.meta_get("generation")?.unwrap_or_default(),"policy":super::policy::VERSION,"requested_mode":if opts.selective{"selective"}else{"full"},"complete":complete,"languages":coverage.values().collect::<Vec<_>>(),"budget_secs":opts.selective.then_some(opts.budget.as_secs())});
    db.meta_set("precision_summary", &summary.to_string())?;
    crate::operation::reason("precision_coverage", summary);
    if !complete {
        crate::operation::partial();
    }
    for message in stats.warnings.iter().chain(&stats.errors) {
        crate::operation::warning(message.clone());
    }
    Ok(stats)
}
fn count_status(c: &mut Coverage, r: &RefRow) {
    match r.precise_status.as_deref() {
        Some("hit" | "external" | "nonode") => c.verified += 1,
        Some(_) => c.unresolved += 1,
        None => {}
    }
}
struct Records {
    generation: String,
    values: HashMap<String, String>,
}
fn record(records: &mut Records, d: &Decision, status: &str, error: Option<&str>) -> Result<()> {
    records.values.insert(format!("precision_ref:{}",d.reference),json!({"policy":super::policy::VERSION,"generation":records.generation,"status":status,"reasons":d.reasons,"error":error}).to_string());
    Ok(())
}
fn failure(
    records: &mut Records,
    decisions: &[Decision],
    code: &str,
    message: &str,
    coverage: &mut BTreeMap<String, Coverage>,
    stats: &mut PreciseStats,
) -> Result<()> {
    for d in decisions {
        let c = coverage.get_mut(&d.language).unwrap();
        c.pending += 1;
        c.failed += usize::from(code != "budget_exhausted");
        c.reasons.push(code.into());
        record(records, d, "pending", Some(code))?;
    }
    let text = format!(
        "{code}[{}]: {message}; {} references remain heuristic/unverified",
        decisions
            .first()
            .map(|d| d.language.as_str())
            .unwrap_or("unknown"),
        decisions.len()
    );
    stats.errors.push(text);
    Ok(())
}
fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}
fn family(language: &str) -> &str {
    match language {
        "c" | "cpp" => "clangd",
        "javascript" | "typescript" => "typescript",
        other => other,
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn error_code(message: &str) -> &'static str {
    if message.contains("unsupported_capability") {
        "unsupported_capability"
    } else if message.contains("protocol_error") || message.contains("bridge payload") {
        "protocol_error"
    } else if message.contains("dependency_missing") {
        "dependency_missing"
    } else if message.contains("starting language server") {
        "runtime_unavailable"
    } else if message.contains("timeout")
        || message.contains("did not answer")
        || message.contains("still indexing")
    {
        "timeout"
    } else if message.contains("exited")
        || message.contains("server_crash")
        || message.contains("lost the connection")
    {
        "server_crash"
    } else if message.contains("initialization") {
        "initialization_failed"
    } else {
        "request_failed"
    }
}
