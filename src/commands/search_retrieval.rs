//! Source-aware Gemma retrieval. Promotion is quality-gated.
use super::{SearchResult, hybrid_lexical_candidates};
use crate::{
    cache::{
        CacheDb,
        models::{FileRow, SymbolRow},
    },
    cli::SearchArgs,
    embeddings::{self, Embedder, Match, SupportingLocation},
    selection::Selection,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub const CANDIDATES: usize = 200;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub promoted: bool,
    pub context_tokens: usize,
    pub ranking: embeddings::context::Ranking,
    pub weak_threshold: Option<f64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            promoted: false,
            context_tokens: 128,
            ranking: Default::default(),
            weak_threshold: None,
        }
    }
}
pub fn profile() -> Options {
    serde_json::from_str(include_str!("search-profile.json"))
        .expect("embedded Gemma search profile")
}
struct Sources<'a> {
    db: &'a CacheDb,
    files: HashMap<String, FileRow>,
    texts: HashMap<String, String>,
}
impl<'a> Sources<'a> {
    fn new(db: &'a CacheDb) -> Result<Self> {
        Ok(Self {
            db,
            files: db
                .all_files()?
                .into_iter()
                .map(|f| (f.path.clone(), f))
                .collect(),
            texts: HashMap::new(),
        })
    }
    fn get(&mut self, path: &str) -> Result<&str> {
        if !self.texts.contains_key(path) {
            let file = self
                .files
                .get(path)
                .with_context(|| format!("source file missing: {path}"))?;
            self.texts
                .insert(path.into(), self.db.source_snapshot(file)?);
        }
        Ok(&self.texts[path])
    }
    fn symbol(&mut self, s: &SymbolRow, path: &str) -> Result<Match> {
        let key = embeddings::chunk::symbol_key(path, s);
        let source = self.get(path)?;
        let start = usize::try_from(s.start_byte.context("symbol has no source start")?)?;
        let end = usize::try_from(s.end_byte.context("symbol has no source end")?)?;
        ensure!(
            source.get(start..end).is_some(),
            "invalid source span for {key}"
        );
        let start = crate::cache::content::doc_start(source, start);
        Ok(source_match(
            key, path, &s.name, &s.kind, source, start, end, "lexical",
        ))
    }
}
#[allow(clippy::too_many_arguments)]
fn source_match(
    key: String,
    path: &str,
    name: &str,
    kind: &str,
    source: &str,
    start: usize,
    end: usize,
    retrieval: &str,
) -> Match {
    let full_end = end;
    let end = source[start..end]
        .char_indices()
        .nth(1600)
        .map_or(end, |(i, _)| start + i);
    Match {
        path: path.into(),
        name: name.into(),
        kind: kind.into(),
        line: source[..start].bytes().filter(|b| *b == b'\n').count() + 1,
        end_line: source[..end].bytes().filter(|b| *b == b'\n').count() + 1,
        start_byte: start,
        end_byte: end,
        snippet: source[start..end].into(),
        symbol_key: key,
        retrieval: retrieval.into(),
        cosine_score: None,
        rrf_score: None,
        relevance_score: None,
        auxiliary_cosine: None,
        supporting_locations: vec![],
        representation: None,
        symbol_range: if end < full_end {
            Some([start, full_end])
        } else {
            None
        },
    }
}
fn content_excerpt(source: &str, start: usize, end: usize, query: &str) -> usize {
    content_excerpt_or(source, start, end, query, start)
}
fn content_excerpt_or(
    source: &str,
    start: usize,
    end: usize,
    query: &str,
    fallback: usize,
) -> usize {
    let wanted: HashSet<_> = crate::cache::content::terms(query).into_iter().collect();
    let mut best = (0usize, start);
    let mut offset = start;
    for line in source[start..end].split_inclusive('\n') {
        let present: HashSet<_> = crate::cache::content::terms(line).into_iter().collect();
        let coverage = wanted.intersection(&present).count();
        if coverage > best.0 {
            best = (coverage, offset);
        }
        offset += line.len();
    }
    if best.0 == 0 {
        return fallback;
    }
    if best.1 == start {
        return start;
    }
    let previous_end = best.1.saturating_sub(1);
    let previous = source[start..previous_end]
        .rfind('\n')
        .map_or(start, |i| start + i + 1);
    if best.1 - previous <= 120 {
        previous
    } else {
        best.1
    }
}
#[derive(Clone)]
struct Neighbor {
    key: String,
    relation: String,
    confidence: f32,
}
struct Relations {
    symbols: HashMap<String, (SymbolRow, String)>,
    neighbors: HashMap<String, Vec<Neighbor>>,
}
type GraphCache = HashMap<(PathBuf, String), Arc<Relations>>;
static GRAPHS: OnceLock<Mutex<GraphCache>> = OnceLock::new();
fn relations(db: &CacheDb, project: &Path) -> Result<Arc<Relations>> {
    let generation = format!(
        "{}:{}",
        db.meta_get("generation")?.unwrap_or_default(),
        db.meta_get("precise_generation")?.unwrap_or_default()
    );
    let project = project.canonicalize()?;
    let key = (project.clone(), generation);
    let mut cache = GRAPHS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("retrieval graph lock poisoned"))?;
    if let Some(value) = cache.get(&key) {
        return Ok(value.clone());
    }
    let _timer = crate::operation::time("retrieval_relations");
    let files: HashMap<_, _> = db
        .all_files()?
        .into_iter()
        .map(|f| (f.id, f.path))
        .collect();
    let mut symbols = HashMap::new();
    let mut ids = HashMap::new();
    for s in db.all_symbols()? {
        let Some(path) = files.get(&s.file_id) else {
            continue;
        };
        let key = embeddings::chunk::symbol_key(path, &s);
        ids.insert(
            crate::graph::symbol_id(path, &s.name, s.start_line.unwrap_or(1)),
            key.clone(),
        );
        symbols.insert(key, (s, path.clone()));
    }
    let graph = crate::graph::resolve::build(
        db,
        &crate::graph::resolve::GraphOptions {
            root: project.to_string_lossy().into_owned(),
            scope: crate::graph::resolve::Scope::Symbol,
            kinds: HashSet::from(["calls".into(), "contains".into()]),
            min_confidence: 0.8,
            include_external: false,
            path_glob: None,
            focus: None,
            depth: 1,
            max_nodes: 0,
        },
    )?;
    let mut neighbors: HashMap<String, Vec<Neighbor>> = HashMap::new();
    for edge in graph.edges {
        let (Some(from), Some(to)) = (ids.get(&edge.source), ids.get(&edge.target)) else {
            continue;
        };
        neighbors.entry(from.clone()).or_default().push(Neighbor {
            key: to.clone(),
            relation: edge.kind.clone(),
            confidence: edge.confidence,
        });
        neighbors.entry(to.clone()).or_default().push(Neighbor {
            key: from.clone(),
            relation: if edge.kind == "contains" {
                "member_of".into()
            } else {
                "called_by".into()
            },
            confidence: edge.confidence,
        });
    }
    for list in neighbors.values_mut() {
        list.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(a.key.cmp(&b.key))
                .then(a.relation.cmp(&b.relation))
        });
        list.dedup_by(|a, b| a.key == b.key && a.relation == b.relation);
    }
    let result = Arc::new(Relations { symbols, neighbors });
    cache.retain(|(p, _), _| p != &project);
    cache.insert(key, result.clone());
    Ok(result)
}
fn accepts(args: &SearchArgs, selection: &Selection, s: &SymbolRow, path: &str) -> bool {
    selection.accepts(path)
        && args.kind.as_ref().is_none_or(|k| k == &s.kind)
        && (!args.exported || s.is_exported)
}
fn merge(branches: Vec<Vec<Match>>) -> Vec<Match> {
    let mut combined: HashMap<String, Match> = HashMap::new();
    for branch in branches {
        for (rank, mut hit) in branch.into_iter().enumerate() {
            let weight = 1.0 / (61.0 + rank as f64);
            if let Some(old) = combined.get_mut(&hit.symbol_key) {
                old.rrf_score = Some(old.rrf_score.unwrap() + weight);
                old.retrieval = "hybrid".into();
                if old.cosine_score.is_none() {
                    old.cosine_score = hit.cosine_score;
                }
            } else {
                hit.rrf_score = Some(weight);
                combined.insert(hit.symbol_key.clone(), hit);
            }
        }
    }
    let mut hits = combined.into_values().collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.rrf_score
            .unwrap()
            .total_cmp(&a.rrf_score.unwrap())
            .then(a.path.cmp(&b.path))
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.symbol_key.cmp(&b.symbol_key))
    });
    hits
}
fn supports(
    hit: &Match,
    relations: &Relations,
    sources: &mut Sources<'_>,
    args: &SearchArgs,
    selection: &Selection,
) -> Result<Vec<SupportingLocation>> {
    let mut result = vec![];
    let mut seen = HashSet::new();
    for neighbor in relations
        .neighbors
        .get(&hit.symbol_key)
        .into_iter()
        .flatten()
    {
        if !seen.insert(&neighbor.key) {
            continue;
        }
        let Some((s, path)) = relations.symbols.get(&neighbor.key) else {
            continue;
        };
        if !sources.files.contains_key(path) || !accepts(args, selection, s, path) {
            continue;
        }
        let related = sources.symbol(s, path)?;
        // Excerpts are exact source prefixes with their own corresponding ranges.
        let finish = related
            .snippet
            .char_indices()
            .nth(600)
            .map_or(related.end_byte, |(i, _)| related.start_byte + i);
        let source = sources.get(path)?;
        result.push(SupportingLocation {
            path: path.clone(),
            name: s.name.clone(),
            kind: s.kind.clone(),
            line: related.line,
            end_line: source[..finish].bytes().filter(|b| *b == b'\n').count() + 1,
            start_byte: related.start_byte,
            end_byte: finish,
            snippet: source[related.start_byte..finish].into(),
            relation: neighbor.relation.clone(),
            confidence: neighbor.confidence,
            resolution_mode: if matches!(neighbor.relation.as_str(), "contains" | "member_of") {
                "syntax".into()
            } else {
                "heuristic".into()
            },
        });
        if result.len() == 50 {
            break;
        }
    }
    let wanted: HashSet<_> = crate::cache::content::terms(&args.query)
        .into_iter()
        .collect();
    let mut scored = result
        .into_iter()
        .map(|s| {
            let terms: HashSet<_> =
                crate::cache::content::terms(&format!("{} {}", s.name, s.snippet))
                    .into_iter()
                    .collect();
            (wanted.intersection(&terms).count(), s)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.confidence.total_cmp(&a.1.confidence))
            .then(a.1.path.cmp(&b.1.path))
            .then(a.1.start_byte.cmp(&b.1.start_byte))
            .then(a.1.relation.cmp(&b.1.relation))
    });
    Ok(scored.into_iter().take(3).map(|(_, s)| s).collect())
}

pub(crate) fn scoped_state(
    db: &CacheDb,
    project: &Path,
    selection: &Selection,
) -> Result<(usize, usize)> {
    let config = crate::config::ProjectConfig::load(project)?;
    let key = embeddings::config_key(&config.embeddings);
    let mut pending = 0;
    let mut blocked = 0;
    for file in embeddings::scope(db, &config)?
        .into_iter()
        .filter(|f| selection.accepts(&f.path))
    {
        if !file.parsed_ok {
            blocked += 1;
            continue;
        }
        if db
            .meta_get(&format!("embedding_file:{}", file.path))?
            .as_deref()
            != Some(format!("{key}:{}", file.content_hash).as_str())
        {
            pending += 1;
        }
    }
    Ok((pending, blocked))
}
pub(crate) fn refresh_index(
    db: &mut CacheDb,
    project: &Path,
    args: &SearchArgs,
    embedder: &mut dyn Embedder,
) -> Result<Option<serde_json::Value>> {
    if args.no_sync {
        return Ok(None);
    }
    let selection = Selection::new(project, &args.filters)?;
    if scoped_state(db, project, &selection)?.0 == 0 {
        return Ok(None);
    }
    let mut guard = crate::progress::Guard::start(project, None)?;
    crate::progress::phase("embedding_refresh");
    let mut result =
        embeddings::index_selected_with(db, project, false, embedder, Some(&selection))?;
    crate::operation::check()?;
    result["operation"] = guard.finish("completed")?;
    Ok(Some(result))
}

pub(crate) fn execute_with(
    db: &mut CacheDb,
    project: &Path,
    args: &SearchArgs,
    embedder: &mut dyn Embedder,
    options: &Options,
) -> Result<SearchResult> {
    execute_traced(db, project, args, embedder, options, None)
}

#[derive(Default, Serialize)]
pub(crate) struct Trace {
    pub candidates: Vec<Match>,
    pub scored: Vec<Match>,
}

fn execute_traced(
    db: &mut CacheDb,
    project: &Path,
    args: &SearchArgs,
    embedder: &mut dyn Embedder,
    options: &Options,
    trace: Option<&mut Trace>,
) -> Result<SearchResult> {
    ensure!(
        (1..=100).contains(&args.limit),
        "semantic/hybrid limit must be 1..=100"
    );
    ensure!(
        (64..=510).contains(&options.context_tokens)
            && [
                options.ranking.function_body_weight,
                options.ranking.container_body_weight,
                options.ranking.query_view_weight,
                options
                    .ranking
                    .purpose_query_view_weight
                    .unwrap_or(options.ranking.query_view_weight),
            ]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "unsupported Gemma context profile"
    );
    let selection = Selection::new(project, &args.filters)?;
    let hybrid = args.hybrid || (!args.semantic && !args.lexical);
    let base_refresh = refresh_index(db, project, args, embedder)?;
    let purpose_refresh = if !args.no_sync
        && !embeddings::context::pending(db, project, &selection, options.context_tokens)?
            .is_empty()
    {
        let mut guard = crate::progress::Guard::start(project, None)?;
        let value = embeddings::context::refresh(
            db,
            project,
            &selection,
            embedder,
            options.context_tokens,
        )?;
        guard.finish("completed")?;
        Some(value)
    } else {
        None
    };
    let context_pending =
        embeddings::context::pending(db, project, &selection, options.context_tokens)?.len();
    if context_pending > 0 {
        crate::operation::embedding_partial();
        crate::operation::warning(
            "purpose context index is incomplete; sync without --no-sync to update it",
        );
    }
    let (scope_pending, scope_blocked) = scoped_state(db, project, &selection)?;
    let all = embeddings::semantic_context_with(
        db,
        project,
        args,
        embedder,
        usize::MAX,
        options.ranking.query_view_weight > 0.0
            || options
                .ranking
                .purpose_query_view_weight
                .is_some_and(|v| v > 0.0),
        Some((&options.ranking, options.context_tokens)),
    )?;
    let evidence: HashMap<_, _> = all
        .iter()
        .map(|h| {
            (
                h.symbol_key.clone(),
                (h.cosine_score, h.auxiliary_cosine, h.relevance_score),
            )
        })
        .collect();
    let semantic = all.into_iter().take(CANDIDATES).collect::<Vec<_>>();
    let semantic_count = semantic.len();
    let cfg = crate::config::ProjectConfig::load(project)?;
    let allowed: HashSet<_> = embeddings::scope(db, &cfg)?
        .into_iter()
        .filter(|f| f.parsed_ok)
        .map(|f| f.path)
        .collect();
    let mut sources = Sources::new(db)?;
    sources
        .files
        .retain(|path, _| allowed.contains(path) && selection.accepts(path));
    let content = db
        .content_candidates(
            &args.query,
            |d| {
                d.kind != "file"
                    && allowed.contains(&d.path)
                    && selection.accepts(&d.path)
                    && args.kind.as_ref().is_none_or(|k| k == &d.kind)
                    && (!args.exported || d.is_exported)
            },
            CANDIDATES,
        )?
        .into_iter()
        .map(|(d, _)| {
            let source = sources.get(&d.path)?;
            ensure!(
                source.get(d.start_byte..d.end_byte).is_some(),
                "invalid content source range"
            );
            let start = content_excerpt(source, d.start_byte, d.end_byte, &args.query);
            let mut h = source_match(
                d.key, &d.path, &d.name, &d.kind, source, start, d.end_byte, "content",
            );
            if h.start_byte != d.start_byte || h.end_byte != d.end_byte {
                h.symbol_range = Some([d.start_byte, d.end_byte]);
            }
            Ok(h)
        })
        .collect::<Result<Vec<_>>>()?;
    let content_count = content.len();
    let lexical = if hybrid {
        hybrid_lexical_candidates(db, args, &selection)?
            .into_iter()
            .filter(|(_, p)| allowed.contains(p))
            .take(CANDIDATES)
            .map(|(s, p)| sources.symbol(&s, &p))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    let lexical_keys: HashSet<_> = lexical.iter().map(|h| h.symbol_key.clone()).collect();
    let lexical_count = lexical.len();
    let mut branches = vec![semantic, content, lexical];
    let seeds = merge(branches.clone());
    let relations = relations(db, project)?;
    let mut seen: HashSet<_> = seeds.iter().map(|h| h.symbol_key.clone()).collect();
    let mut expanded = vec![];
    'seeds: for seed in seeds.iter().take(32) {
        for n in relations
            .neighbors
            .get(&seed.symbol_key)
            .into_iter()
            .flatten()
        {
            if seen.contains(&n.key) {
                continue;
            }
            let Some((s, path)) = relations.symbols.get(&n.key) else {
                continue;
            };
            if !allowed.contains(path) || !accepts(args, &selection, s, path) {
                continue;
            }
            seen.insert(n.key.clone());
            let mut h = sources.symbol(s, path)?;
            h.retrieval = "graph".into();
            expanded.push(h);
            if expanded.len() == 50 {
                break 'seeds;
            }
        }
    }
    let graph_count = expanded.len();
    branches.push(expanded);
    let mut hits = merge(branches);
    for hit in &mut hits {
        if let Some(&(primary, secondary, relevance)) = evidence.get(&hit.symbol_key) {
            hit.cosine_score = primary;
            hit.auxiliary_cosine = secondary;
            hit.relevance_score = relevance;
        }
    }
    let query = args.query.trim().trim_matches('`');
    let identity = !query.is_empty() && query.chars().all(|c| c.is_alphanumeric() || c == '_');
    hits.sort_by(|a, b| {
        (identity && b.name == query)
            .cmp(&(identity && a.name == query))
            .then_with(|| {
                lexical_keys
                    .contains(&b.symbol_key)
                    .cmp(&lexical_keys.contains(&a.symbol_key))
            })
            .then_with(|| {
                b.relevance_score
                    .unwrap_or(-1.0)
                    .total_cmp(&a.relevance_score.unwrap_or(-1.0))
            })
            .then(a.path.cmp(&b.path))
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.symbol_key.cmp(&b.symbol_key))
    });
    let total = hits.len();
    if let Some(trace) = trace {
        trace.candidates = hits.clone();
        trace.scored = hits
            .iter()
            .filter(|h| h.relevance_score.is_some())
            .cloned()
            .collect();
    }
    hits.truncate(args.limit);
    for hit in &mut hits {
        hit.supporting_locations = supports(hit, &relations, &mut sources, args, &selection)?;
    }
    let quality = if hits.is_empty() {
        "empty"
    } else if options
        .weak_threshold
        .is_some_and(|t| hits[0].relevance_score.is_none_or(|s| s < t))
    {
        "weak"
    } else {
        "candidate"
    };
    crate::operation::omitted(total.saturating_sub(args.limit));
    crate::operation::retrieval(
        json!({"mode":if hybrid{"hybrid"}else{"semantic"},"selection_reason":super::mode_reason(args,true),"ranking_model":embeddings::MODEL,"ranking_revision":embeddings::REVISION,"ranking_method":"gemma_source_and_purpose","context_version":embeddings::context::VERSION,"context_tokens":options.context_tokens,"ranking":options.ranking,"semantic_candidates":semantic_count,"content_candidates":content_count,"lexical_candidates":lexical_count,"graph_candidates":graph_count,"fused_candidates":total,"candidate_limit_per_branch":CANDIDATES,"total_matches_scope":"candidate_pool","quality":quality,"index_refresh":base_refresh,"context_refresh":purpose_refresh,"source_verified":!args.no_sync,"scope_complete":scope_pending==0&&scope_blocked==0&&context_pending==0,"scope_pending_files":scope_pending,"scope_blocked_files":scope_blocked,"scope_pending_context_files":context_pending}),
    );
    Ok(SearchResult {
        matches: Some(hits),
        total_matches: total,
        match_counts: BTreeMap::from([("candidates".into(), total)]),
        quality: Some(quality.into()),
        query: args.query.clone(),
        exact_symbols: vec![],
        string_literals: vec![],
        fuzzy_suggestions: vec![],
        grep_matches: vec![],
    })
}

#[cfg(all(test, feature = "embeddings"))]
#[path = "search_evaluation.rs"]
mod evaluation;
