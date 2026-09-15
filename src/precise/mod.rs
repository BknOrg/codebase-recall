//! Ground-truth reference resolution through real language servers.
//!
//! The default pipeline infers where a reference points from the AST alone,
//! which cannot follow generics, trait/interface dispatch or overloads. This
//! pass asks the compiler-grade tool for each language instead —
//! rust-analyzer, Pyright, Eclipse JDT LS, kotlin-language-server — for the one
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
mod map;

use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::cache::CacheDb;
use crate::cache::models::{FileRow, PreciseStatus, SymbolRow};
use backend::Backend;
use client::{LspClient, Readiness};
use map::{PositionMap, path_to_uri, symbol_at_line, uri_to_path};

pub struct PreciseOptions {
    /// Re-ask about every file, not just the ones with no answers yet.
    pub full: bool,
    /// Budget for a single request to the server.
    pub request_timeout: Duration,
    /// `--language` tokens, empty for "every language that has a backend".
    pub language_filter: Vec<String>,
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
    let mut stats = PreciseStats::default();

    // ---- preconditions -----------------------------------------------------
    ensure!(
        project.is_dir(),
        "project directory `{}` does not exist (or is not a directory)",
        project.display()
    );
    // Two forms of the root, both needed: the canonical one to compare against
    // canonicalized paths the server returns, and a plain absolute one to hand
    // *to* the server, since Windows' `\\?\` prefix is not valid inside a URI.
    let canonical_root = fs::canonicalize(project)
        .with_context(|| format!("resolving the full path of `{}`", project.display()))?;
    let server_root = map::strip_extended_prefix(&canonical_root);

    let files = db.all_files()?;
    if files.is_empty() {
        stats.warnings.push(
            "the cache holds no analyzed files, so there is nothing to resolve. \
             Run `code-rcl sync` first."
                .to_string(),
        );
        return Ok(stats);
    }

    let mut by_language: HashMap<&str, Vec<&FileRow>> = HashMap::new();
    for f in &files {
        by_language.entry(f.language.as_str()).or_default().push(f);
    }

    let selected: Vec<(&'static Backend, Vec<&FileRow>)> = backend::BACKENDS
        .iter()
        .filter(|b| language_selected(b.lang_group, &opts.language_filter))
        .filter_map(|b| {
            by_language
                .get(b.lang_group)
                .map(|files| (b, files.clone()))
        })
        .collect();

    if selected.is_empty() {
        let mut present: Vec<&str> = by_language.keys().copied().collect();
        present.sort_unstable();
        stats.warnings.push(format!(
            "no file in this project belongs to a language `--precise` can resolve.\n  \
             Supported: {}.\n  \
             Found here: {}.",
            backend::supported_languages(),
            if present.is_empty() {
                "(none)".to_string()
            } else {
                present.join(", ")
            },
        ));
        return Ok(stats);
    }

    // Indexes shared by every language's pass.
    let symbols = db.all_symbols()?;
    let mut symbols_by_file: HashMap<i64, Vec<&SymbolRow>> = HashMap::new();
    for s in &symbols {
        symbols_by_file.entry(s.file_id).or_default().push(s);
    }
    let file_id_by_path: HashMap<String, i64> =
        files.iter().map(|f| (f.path.clone(), f.id)).collect();

    for (backend, lang_files) in selected {
        let launcher = match backend.locate() {
            Ok(l) => l,
            Err(why) => {
                stats.warnings.push(format!(
                    "skipping {} — {why}\n  \
                     The heuristic edges for {} are kept as they are.",
                    backend.lang_group, backend.lang_group
                ));
                continue;
            }
        };

        if backend.lacks_project_model(&server_root) {
            stats.warnings.push(format!(
                "{}: no {} found in the project root — {}.",
                backend.lang_group,
                backend.project_markers.join(" / "),
                backend.marker_hint
            ));
        }

        let pending: Vec<&FileRow> = lang_files
            .into_iter()
            .filter(|f| opts.full || f.precise_synced_at.is_none())
            .collect();
        if pending.is_empty() {
            continue;
        }

        if opts.full {
            let ids: Vec<i64> = pending.iter().map(|f| f.id).collect();
            db.clear_precise(&ids)?;
        }

        let mut resolver = Resolver {
            db: &mut *db,
            project: &server_root,
            canonical_root: &canonical_root,
            symbols_by_file: &symbols_by_file,
            file_id_by_path: &file_id_by_path,
            uri_cache: HashMap::new(),
            opts,
        };

        match resolver.run_language(backend, &launcher, &pending) {
            Ok((outcome, warnings)) => {
                stats.warnings.extend(warnings);
                stats.languages.push(outcome);
            }
            Err(e) => stats
                .errors
                .push(format!("{} ({}): {e:#}", backend.lang_group, launcher.name)),
        }
    }

    Ok(stats)
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
        client.initialize(self.project, Duration::from_secs(60))?;

        eprintln!(
            "precise[{}]: {} indexing {} file(s)...",
            backend.lang_group,
            client.name(),
            files.len()
        );
        if client.wait_until_ready(Duration::from_secs(backend.index_timeout_secs))
            == Readiness::TimedOut
        {
            warnings.push(format!(
                "{}: {} was still indexing after {}s, so some answers may be incomplete. \
                 Re-run `code-rcl sync --precise` once it has warmed up.",
                backend.lang_group, launcher.name, backend.index_timeout_secs
            ));
        }

        let result = self.resolve_files(&mut client, backend, files, &mut outcome, &mut warnings);
        client.shutdown();
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
        let show_progress = std::io::stderr().is_terminal();

        for (i, file) in files.iter().enumerate() {
            let absolute = self.project.join(&file.path);
            let text = match fs::read(&absolute) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => {
                    warnings.push(format!(
                        "{}: could not read `{}` ({e}); its references keep their heuristic edges.",
                        backend.lang_group, file.path
                    ));
                    continue;
                }
            };

            // The stored byte offsets describe the text as it was at sync time.
            // If the file moved on since, pointing the server at those offsets
            // would resolve the wrong tokens, so leave it for the next sync.
            let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
            if hash != file.content_hash {
                warnings.push(format!(
                    "{}: `{}` changed since it was analyzed; skipping it. \
                     Re-run `code-rcl sync --precise` to pick it up.",
                    backend.lang_group, file.path
                ));
                continue;
            }

            let refs = self.db.refs_in_file(file.id)?;
            if refs.is_empty() {
                self.db.set_precise_results(file.id, &[])?;
                outcome.files += 1;
                continue;
            }

            let positions = PositionMap::new(text);
            let uri = path_to_uri(&absolute);
            client.did_open(&uri, backend.language_id, positions.text())?;

            let mut results = Vec::with_capacity(refs.len());
            for reference in &refs {
                // Only the four precise-capable analyzers record the name token.
                let Some(offset) = reference.name_start_byte else {
                    continue;
                };
                let Some((line, character)) = positions.position(offset as usize) else {
                    continue;
                };

                let answer = client.definition(&uri, line, character, self.opts.request_timeout)?;
                let (status, symbol_id, confidence) = self.classify(&answer);
                match status {
                    PreciseStatus::Hit => outcome.hits += 1,
                    PreciseStatus::External => outcome.external += 1,
                    PreciseStatus::NoNode => outcome.nonode += 1,
                    PreciseStatus::Unresolved => outcome.unresolved += 1,
                }
                outcome.queried += 1;
                results.push((reference.id, status, symbol_id, confidence));
            }

            client.did_close(&uri)?;
            self.db.set_precise_results(file.id, &results)?;
            outcome.files += 1;

            if show_progress {
                eprint!(
                    "\rprecise[{}]: {}/{} files, {} refs resolved",
                    backend.lang_group,
                    i + 1,
                    files.len(),
                    outcome.hits
                );
            }
        }
        if show_progress && !files.is_empty() {
            eprintln!();
        }
        Ok(())
    }

    /// Turn a `textDocument/definition` answer into something storable.
    fn classify(&mut self, answer: &Value) -> (PreciseStatus, Option<i64>, f64) {
        let targets = definition_targets(answer);
        if targets.is_empty() {
            return (PreciseStatus::Unresolved, None, 0.0);
        }

        let mut hits: Vec<i64> = Vec::new();
        let mut saw_outside = false;
        for (uri, line) in &targets {
            match self.file_id_for_uri(uri) {
                Some(file_id) => {
                    let symbols = self.symbols_by_file.get(&file_id);
                    if let Some(symbol) = symbols.and_then(|s| symbol_at_line(s, *line))
                        && !hits.contains(&symbol.id)
                    {
                        hits.push(symbol.id);
                    }
                }
                None => saw_outside = true,
            }
        }

        match hits.len() {
            0 if saw_outside => (PreciseStatus::External, None, 0.0),
            0 => (PreciseStatus::NoNode, None, 0.0),
            // Exactly one definition: the compiler's own answer.
            1 => (PreciseStatus::Hit, Some(hits[0]), 1.0),
            // Several (a trait method and its impls, say). The first is what an
            // editor's "go to definition" jumps to, but it is a choice, so the
            // edge is marked slightly below certain.
            _ => (PreciseStatus::Hit, Some(hits[0]), 0.9),
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
fn definition_targets(answer: &Value) -> Vec<(String, i64)> {
    match answer {
        Value::Array(items) => items.iter().filter_map(single_target).collect(),
        Value::Object(_) => single_target(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn single_target(item: &Value) -> Option<(String, i64)> {
    // A LocationLink points at the definition's name range, which is more
    // precise than the whole definition body a plain Location carries.
    if let Some(uri) = item.get("targetUri").and_then(Value::as_str) {
        let range = item
            .get("targetSelectionRange")
            .or_else(|| item.get("targetRange"))?;
        return Some((uri.to_string(), line_of(range)?));
    }
    let uri = item.get("uri").and_then(Value::as_str)?;
    Some((uri.to_string(), line_of(item.get("range")?)?))
}

/// LSP lines are 0-based; symbol rows are 1-based.
fn line_of(range: &Value) -> Option<i64> {
    range
        .get("start")
        .and_then(|s| s.get("line"))
        .and_then(Value::as_i64)
        .map(|l| l + 1)
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
            vec![("file:///p/src/util.rs".to_string(), 10)]
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
            vec![("file:///p/src/util.rs".to_string(), 6)]
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
