//! Ground-truth reference resolution through real language servers.
//!
//! The default pipeline infers where a reference points from the AST alone,
//! which cannot follow generics, trait/interface dispatch or overloads. This
//! pass asks the compiler-grade tool for each language instead —
//! rust-analyzer, Pyright, Eclipse JDT LS, intellij-server / kotlin-language-server — for the one
//! question that matters here: *where is this name defined?*
//!
//! The answer is stored per reference and read back by the graph resolver as
//! its top layer. It is authoritative in both directions: it adds edges the
//! heuristics miss, and suppresses ones they would invent when the real
//! definition turns out to live in the standard library or a dependency.
//!
//! Nothing here is bundled. A missing server is reported with install
//! instructions and the language is skipped, leaving the heuristic edges in
//! place — `--precise` improves the graph, it never empties it.

pub mod backend;
mod client;
mod engine;
pub(crate) mod map;
pub(crate) mod policy;
mod process;
pub(crate) mod state;
mod vue;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::cache::CacheDb;
use crate::cache::models::{FileRow, PreciseStatus, SymbolRow};
use backend::Backend;
use client::{LspClient, Readiness};
use map::{PositionMap, path_to_uri, symbol_at_byte, uri_to_path};

pub struct PreciseOptions {
    pub selective: bool,
    pub budget: Duration,
    /// Re-ask about every file, not just the ones with no answers yet.
    pub full: bool,
    /// Budget for a single request to the server.
    pub request_timeout: Duration,
    /// `--language` tokens, empty for "every language that has a backend".
    pub language_filter: Vec<String>,
    /// The backends to use, with the Kotlin one chosen by
    /// `[precise.kotlin] server` (see [`backend::select_backends`]).
    pub backends: Vec<&'static Backend>,
}

/// What one language's pass achieved, for the summary line.
pub struct LanguageOutcome {
    pub language: String,
    pub server: String,
    pub files: usize,
    pub queried: usize,
    pub hits: usize,
    pub external: usize,
    pub nonode: usize,
    pub unresolved: usize,
}

#[derive(Default)]
pub struct PreciseStats {
    pub languages: Vec<LanguageOutcome>,
    /// Conditions the user should know about that did not stop the run:
    /// a missing server, a project without a build file, a skipped file.
    pub warnings: Vec<String>,
    /// Failures that cut a language's pass short. Results gathered before the
    /// failure are still saved.
    pub errors: Vec<String>,
}

/// Resolve every pending reference through the language server of its language.
///
/// Returns `Ok` even when individual languages fail; inspect
/// [`PreciseStats::errors`] to decide the exit code.
pub fn run_precise_pass(
    db: &mut CacheDb,
    project: &Path,
    opts: &PreciseOptions,
) -> Result<PreciseStats> {
    engine::run(db, project, opts)
}

fn language_selected(lang_group: &str, filter: &[String]) -> bool {
    filter.is_empty()
        || filter.iter().any(|t| {
            let t = t.trim().to_ascii_lowercase();
            match t.as_str() {
                "py" => lang_group == "python",
                "kt" => lang_group == "kotlin",
                "rs" => lang_group == "rust",
                "ts" => lang_group == "typescript",
                "js" => lang_group == "javascript",
                "c++" => lang_group == "cpp",
                other => other == lang_group,
            }
        })
}

struct Resolver<'a> {
    db: &'a mut CacheDb,
    project: &'a Path,
    canonical_root: &'a Path,
    symbols_by_file: &'a HashMap<i64, Vec<&'a SymbolRow>>,
    file_id_by_path: &'a HashMap<String, i64>,
    /// Definition URIs repeat constantly; resolving one costs a `canonicalize`.
    uri_cache: HashMap<String, Option<i64>>,
    opts: &'a PreciseOptions,
    selected: Option<&'a HashMap<i64, usize>>,
    deadline: Option<Instant>,
    cfg: &'a crate::config::ProjectConfig,
}

impl Resolver<'_> {
    fn run_language(
        &mut self,
        backend: &Backend,
        launcher: &backend::Launcher,
        files: &[&FileRow],
    ) -> Result<(LanguageOutcome, Vec<String>)> {
        let mut warnings = Vec::new();
        let mut outcome = LanguageOutcome {
            language: backend.lang_group.to_string(),
            server: launcher.name.clone(),
            files: 0,
            queried: 0,
            hits: 0,
            external: 0,
            nonode: 0,
            unresolved: 0,
        };

        let mut client = LspClient::start(launcher, self.project)?;
        if let Some(deadline) = self.deadline {
            client.set_deadline(deadline);
        }
        if backend.lang_group == "vue" {
            client.attach_bridge(vue::companion(
                self.project,
                launcher,
                self.cfg,
                self.deadline,
                self.opts.request_timeout,
            )?);
        }
        client.initialize(self.project, self.opts.request_timeout)?;
        // Keep translation units open before any header queries, including on warm passes.
        if matches!(backend.lang_group, "c" | "cpp") {
            let context =
                crate::c_family::BuildContext::load_cached(self.project, self.cfg, self.db)?;
            let mut units = std::collections::BTreeMap::new();
            for file in files {
                let path = self.project.join(&file.path);
                if crate::c_family::is_header(&path) {
                    if let Some(header) = context.headers.get(&crate::c_family::absolute(&path))
                        && let Some(includer) = &header.includer
                    {
                        units.insert(
                            includer.clone(),
                            context
                                .language(includer)
                                .map(|l| l.group())
                                .unwrap_or("cpp"),
                        );
                    }
                } else {
                    units.insert(path, file.language.as_str());
                }
            }
            for (path, language) in units {
                crate::operation::check()?;
                let text = fs::read_to_string(&path)?;
                client.did_open(&path_to_uri(&path), language, &text)?;
            }
        }

        eprintln!(
            "precise[{}]: {} indexing {} file(s)...",
            backend.lang_group,
            client.name(),
            files.len()
        );
        let indexing_started = Instant::now();
        let indexing_budget =
            Duration::from_secs(backend.index_timeout_secs).min(self.opts.request_timeout);
        let readiness = client.wait_until_ready(indexing_budget);
        eprintln!(
            "precise[{}]: {} indexing wait ended after {:.1}s",
            backend.lang_group,
            client.name(),
            indexing_started.elapsed().as_secs_f32()
        );
        if readiness == Readiness::TimedOut {
            anyhow::bail!(
                "{}: {} was still indexing after {}s, so some answers may be incomplete. \
                 Re-run `code-rcl sync --precise` once it has warmed up.",
                backend.lang_group,
                launcher.name,
                indexing_budget.as_secs()
            );
        }

        let result = self.resolve_files(&mut client, backend, files, &mut outcome, &mut warnings);
        let cleanup = crate::operation::time("precise_cleanup");
        client.shutdown();
        drop(cleanup);
        result?;

        Ok((outcome, warnings))
    }

    fn resolve_files(
        &mut self,
        client: &mut LspClient,
        backend: &Backend,
        files: &[&FileRow],
        outcome: &mut LanguageOutcome,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        for file in files {
            crate::operation::check()?;
            if self.deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let result = (|| -> Result<()> {
                let absolute = self.project.join(&file.path);
                let text = fs::read_to_string(&absolute)?;
                ensure!(
                    blake3::hash(text.as_bytes()).to_hex().as_str() == file.content_hash,
                    "{} changed during precise resolution; sync again",
                    file.path
                );
                let references = self.db.refs_in_file(file.id)?;
                let mut references: Vec<_> = references
                    .into_iter()
                    .filter(|r| self.selected.is_none_or(|ids| ids.contains_key(&r.id)))
                    .collect();
                if let Some(ranks) = self.selected {
                    references.sort_by_key(|r| ranks.get(&r.id).copied().unwrap_or(usize::MAX));
                }
                if references.is_empty() {
                    if !self.opts.selective {
                        self.db.set_precise_results(file.id, &[])?;
                    }
                    return Ok(());
                }
                let positions = PositionMap::new(text);
                let uri = path_to_uri(&absolute);
                let language = document_language(file, backend);
                if !matches!(backend.lang_group, "c" | "cpp")
                    || crate::c_family::is_header(&absolute)
                {
                    client.open_bridge(&uri, language, positions.text())?;
                    client.did_open(&uri, language, positions.text())?;
                }
                let mut ids = Vec::new();
                let mut points = Vec::new();
                for r in &references {
                    if let Some(position) = r
                        .name_start_byte
                        .and_then(|o| positions.position(o as usize))
                    {
                        ids.push(r.id);
                        points.push(position);
                    }
                }
                let answers = if self.opts.selective {
                    client.definitions_partial(&uri, &points, self.opts.request_timeout)?
                } else {
                    client
                        .definitions(&uri, &points, self.opts.request_timeout)?
                        .into_iter()
                        .map(Ok)
                        .collect()
                };
                let source_errors = if matches!(backend.lang_group, "c" | "cpp") {
                    client.errors(&uri)
                } else {
                    Vec::new()
                };
                if !self.opts.selective {
                    ensure!(
                        source_errors.is_empty(),
                        "clangd reported errors in {}: {}; fix build context or source before precise",
                        file.path,
                        source_errors.join("; ")
                    );
                }
                let mut results = Vec::new();
                let mut coverage = Vec::new();
                for (id, answer) in ids.into_iter().zip(answers) {
                    let answer = if source_errors.is_empty() {
                        answer
                    } else {
                        Err(format!("source_diagnostics: {}", source_errors.join("; ")))
                    };
                    match answer {
                        Ok(answer) => {
                            let (status, symbol, confidence) = self.classify(&answer);
                            match status {
                                PreciseStatus::Hit => outcome.hits += 1,
                                PreciseStatus::External => outcome.external += 1,
                                PreciseStatus::NoNode => outcome.nonode += 1,
                                PreciseStatus::Unresolved => outcome.unresolved += 1,
                                PreciseStatus::Ambiguous => {
                                    outcome.unresolved += 1;
                                    warnings.push(format!("{}: LSP returned multiple distinct targets for reference {id}; left unresolved",file.path));
                                }
                            }
                            outcome.queried += 1;
                            results.push((id, status, symbol, confidence));
                            coverage.push((
                                id,
                                serde_json::json!({"status":"checked","attempted":true})
                                    .to_string(),
                            ));
                        }
                        Err(error) => {
                            let attempted = error != "pending";
                            outcome.queried += usize::from(attempted);
                            warnings.push(format!("{}:{id}: {error}", file.path));
                            coverage.push((id,serde_json::json!({"status":"failed","attempted":attempted,"error":error}).to_string()));
                        }
                    }
                }
                self.db.set_precise_subset(
                    file.id,
                    &results,
                    !self.opts.selective && results.len() == references.len(),
                    &coverage,
                )?;
                outcome.files += 1;
                if !matches!(backend.lang_group, "c" | "cpp") {
                    client.did_close(&uri)?;
                }
                Ok(())
            })();
            crate::operation::check()?;
            if let Err(error) = result {
                if self.opts.selective {
                    warnings.push(format!(
                        "file_failed[{}]: {error:#}; other files continue",
                        file.path
                    ));
                } else {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Turn a `textDocument/definition` answer into something storable.
    fn classify(&mut self, answer: &Value) -> (PreciseStatus, Option<i64>, f64) {
        let mut targets = definition_targets(answer);
        targets.sort();
        targets.dedup();
        if targets.len() > 1 {
            return (PreciseStatus::Ambiguous, None, 0.0);
        }
        if targets.is_empty() {
            return (PreciseStatus::Unresolved, None, 0.0);
        }

        let mut hits: Vec<i64> = Vec::new();
        let mut saw_outside = false;
        let mut saw_nonode = false;
        for (uri, line, column) in &targets {
            match self.file_id_for_uri(uri) {
                Some(file_id) => {
                    let symbols = self.symbols_by_file.get(&file_id);
                    let offset = uri_to_path(uri)
                        .and_then(|path| fs::read_to_string(path).ok())
                        .and_then(|text| {
                            PositionMap::new(text).byte_offset((*line - 1) as u32, *column)
                        });
                    if let Some(symbol) = offset
                        .and_then(|offset| symbols.and_then(|s| symbol_at_byte(s, offset as i64)))
                        && !hits.contains(&symbol.id)
                    {
                        hits.push(symbol.id);
                    } else if offset
                        .and_then(|offset| symbols.and_then(|s| symbol_at_byte(s, offset as i64)))
                        .is_none()
                    {
                        saw_nonode = true;
                    }
                }
                None if !uri.starts_with("file:")
                    || uri.contains("__VLS_")
                    || uri.contains(".vue.ts")
                    || uri.contains(".svelte.ts") =>
                {
                    saw_nonode = true
                }
                None => saw_outside = true,
            }
        }

        if targets.len() > 1 && (saw_outside || saw_nonode || hits.len() != 1) {
            return (PreciseStatus::Ambiguous, None, 0.0);
        }
        match hits.len() {
            0 if saw_outside => (PreciseStatus::External, None, 0.0),
            0 => (PreciseStatus::NoNode, None, 0.0),
            // Exactly one definition: the compiler's own answer.
            1 => (PreciseStatus::Hit, Some(hits[0]), 1.0),
            // Multiple distinct definitions must not become a first-target edge.
            _ => (PreciseStatus::Ambiguous, None, 0.0),
        }
    }

    /// The cached file a definition URI refers to, or `None` when it points
    /// outside the project (the standard library, a dependency, a jar).
    fn file_id_for_uri(&mut self, uri: &str) -> Option<i64> {
        if let Some(cached) = self.uri_cache.get(uri) {
            return *cached;
        }
        let resolved = self.lookup_uri(uri);
        self.uri_cache.insert(uri.to_string(), resolved);
        resolved
    }

    fn lookup_uri(&self, uri: &str) -> Option<i64> {
        let absolute = uri_to_path(uri)?;
        // Canonicalized on both sides so symlinks, `..` and Windows path
        // spellings compare equal.
        let canonical = fs::canonicalize(&absolute).unwrap_or(absolute);
        let relative = canonical.strip_prefix(self.canonical_root).ok()?;
        let relative = normalize_separators(relative);
        self.file_id_by_path.get(relative.as_str()).copied()
    }
}

fn normalize_separators(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `(uri, 1-based line)` for every definition in an LSP answer, which may be a
/// single `Location`, a `Location[]`, or a `LocationLink[]`.
fn definition_targets(answer: &Value) -> Vec<(String, i64, u32)> {
    match answer {
        Value::Array(items) => items.iter().filter_map(single_target).collect(),
        Value::Object(_) => single_target(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn single_target(item: &Value) -> Option<(String, i64, u32)> {
    // A LocationLink points at the definition's name range, which is more
    // precise than the whole definition body a plain Location carries.
    if let Some(uri) = item.get("targetUri").and_then(Value::as_str) {
        let range = item
            .get("targetSelectionRange")
            .or_else(|| item.get("targetRange"))?;
        return Some((uri.to_string(), line_of(range)?, column_of(range)?));
    }
    let uri = item.get("uri").and_then(Value::as_str)?;
    Some((
        uri.to_string(),
        line_of(item.get("range")?)?,
        column_of(item.get("range")?)?,
    ))
}

/// LSP lines are 0-based; symbol rows are 1-based.
fn column_of(range: &Value) -> Option<u32> {
    u32::try_from(range.get("start")?.get("character")?.as_u64()?).ok()
}

fn line_of(range: &Value) -> Option<i64> {
    range
        .get("start")
        .and_then(|s| s.get("line"))
        .and_then(Value::as_i64)
        .map(|l| l + 1)
}

fn document_language<'a>(file: &'a FileRow, backend: &Backend) -> &'a str {
    match crate::analysis::Language::from_path(Path::new(&file.path)) {
        Some(crate::analysis::Language::Jsx) => "javascriptreact",
        Some(crate::analysis::Language::Tsx) => "typescriptreact",
        _ if file.language == backend.lang_group => backend.language_id,
        _ => &file.language,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_both_location_and_location_link_answers() {
        let location = json!({
            "uri": "file:///p/src/util.rs",
            "range": { "start": { "line": 9, "character": 3 },
                       "end":   { "line": 9, "character": 8 } },
        });
        assert_eq!(
            definition_targets(&location),
            vec![("file:///p/src/util.rs".to_string(), 10, 3)]
        );

        // A link's selection range (the name) wins over its full range.
        let link = json!([{
            "targetUri": "file:///p/src/util.rs",
            "targetRange": { "start": { "line": 4, "character": 0 },
                             "end":   { "line": 12, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 5, "character": 7 },
                                      "end":   { "line": 5, "character": 11 } },
        }]);
        assert_eq!(
            definition_targets(&link),
            vec![("file:///p/src/util.rs".to_string(), 6, 7)]
        );

        assert!(definition_targets(&Value::Null).is_empty());
        assert!(definition_targets(&json!([])).is_empty());
    }

    #[test]
    fn language_filter_accepts_short_tokens() {
        assert!(language_selected("rust", &[]));
        assert!(language_selected("python", &["py".to_string()]));
        assert!(language_selected("kotlin", &["kt".to_string()]));
        assert!(!language_selected("java", &["rust".to_string()]));
    }
}
