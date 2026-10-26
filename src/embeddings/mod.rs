//! Local semantic retrieval. Source sync invalidates derived chunks; indexing is explicit.
#[cfg(all(test, feature = "embeddings"))]
mod benchmark;
pub mod chunk;
mod compact;
pub(crate) mod context;
#[cfg(feature = "embeddings")]
mod model;
mod scheduler;
mod staging;
#[cfg(test)]
mod tests;
use crate::{
    cache::{CacheDb, models::FileRow},
    config::{EmbeddingsConfig, ProjectConfig},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

pub const MODEL: &str = "embeddinggemma-300m-q4";
pub const REVISION: &str = "5090578d9565bb06545b4552f76e6bc2c93e4a66";
pub const PIPELINE: &str = "symbol-v1-t512-o64-code-retrieval";
pub fn pipeline(cfg: &EmbeddingsConfig) -> &'static str {
    match cfg.chunk_profile {
        crate::config::EmbeddingChunkProfile::Full => PIPELINE,
        crate::config::EmbeddingChunkProfile::Compact => {
            "symbol-v2-compact-t512-o64-code-retrieval"
        }
    }
}
pub use crate::cache::embeddings::Chunk;
pub(crate) fn recover_staging(project: &Path) -> Result<()> {
    if project.join(".code-rcl/embedding-staging").exists() {
        drop(staging::Staging::new(project)?);
    }
    Ok(())
}
/// Offsets are UTF-8 byte offsets, with special-token offsets omitted.
pub trait Embedder {
    fn offsets(&self, text: &str) -> Result<Vec<(usize, usize)>>;
    fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    /// Final model length, including special tokens and excluding padding.
    fn input_tokens(&self, text: &str) -> Result<usize> {
        Ok(self.offsets(text)?.len() + 2)
    }
}
pub fn config_key(cfg: &EmbeddingsConfig) -> String {
    format!("{MODEL}:{REVISION}:{}:{}", cfg.dimensions, pipeline(cfg))
}
pub fn normalize(vector: &[f32], dimensions: usize) -> Result<Vec<f32>> {
    ensure!(
        vector.len() == 768,
        "model must return exactly 768 dimensions"
    );
    ensure!(
        vector.iter().all(|x| x.is_finite()),
        "embedding contains non-finite values"
    );
    ensure!(
        [256, 768].contains(&dimensions),
        "dimensions must be 256 or 768"
    );
    let v = &vector[..dimensions];
    let norm = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    ensure!(
        norm.is_finite() && norm > 1e-12,
        "embedding has zero or invalid norm"
    );
    Ok(v.iter().map(|x| (*x as f64 / norm) as f32).collect())
}
pub(crate) fn scope(db: &CacheDb, cfg: &ProjectConfig) -> Result<Vec<FileRow>> {
    let mut files = Vec::new();
    for f in db.all_files()? {
        let language = crate::analysis::Language::from_path(Path::new(&f.path));
        let selected = language.is_some_and(|l| {
            l.is_supported()
                && (cfg.sync.languages.is_empty()
                    || cfg.sync.languages.iter().any(|t| l.matches_filter(t)))
        });
        if selected
            && f.size.unwrap_or(0).max(0) as u64 <= cfg.sync.max_file_kb.saturating_mul(1024)
            && db
                .meta_get(&format!("embedding_excluded:{}", f.path))?
                .as_deref()
                != Some("true")
        {
            files.push(f);
        }
    }
    Ok(files)
}
pub fn status(db: &CacheDb, project: &Path, verified: bool) -> Result<Value> {
    let _timer = crate::operation::time("embedding_status");
    let cfg = ProjectConfig::load(project)?;
    let key = config_key(&cfg.embeddings);
    let files = scope(db, &cfg)?;
    let expected: HashMap<_, _> = files
        .iter()
        .map(|f| (f.path.as_str(), f.content_hash.as_str()))
        .collect();
    let mut pending: usize = 0;
    for f in &files {
        if !f.parsed_ok
            || db
                .meta_get(&format!("embedding_file:{}", f.path))?
                .as_deref()
                != Some(format!("{key}:{}", f.content_hash).as_str())
        {
            pending += 1;
        }
    }
    let blocked = files.iter().filter(|f| !f.parsed_ok).count();
    let (valid, invalid_chunks) = if db.meta_get("embedding_count_format")?.as_deref() == Some("1")
    {
        let mut total = 0;
        let mut valid = 0;
        for file in db.all_files()? {
            if let Some(raw) = db.meta_get(&format!("embedding_count:{}", file.path))? {
                let count: Value = serde_json::from_str(&raw)?;
                total += count["total"].as_u64().context("invalid chunk count")? as usize;
                if expected
                    .get(file.path.as_str())
                    .is_some_and(|hash| Some(*hash) == count["source_hash"].as_str())
                    && count["config_key"] == key
                    && file.parsed_ok
                    && db
                        .meta_get(&format!("embedding_file:{}", file.path))?
                        .as_deref()
                        == Some(format!("{key}:{}", file.content_hash).as_str())
                {
                    valid += count["valid"]
                        .as_u64()
                        .context("invalid valid-chunk count")?
                        as usize;
                }
            }
        }
        ensure!(
            valid <= total,
            "invalid embedding counters; rebuild embedding index"
        );
        (valid, total - valid)
    } else {
        // Older 0.10 caches remain readable; next explicit sync upgrades counters.
        let rows = db.embedding_headers()?;
        let valid = rows
            .iter()
            .filter(|r| {
                r.valid
                    && r.config_key == key
                    && expected
                        .get(r.path.as_str())
                        .is_some_and(|h| *h == r.source_hash)
            })
            .count();
        (valid, rows.len() - valid)
    };
    let complete = pending == 0 && db.meta_get("embedding_complete")?.as_deref() == Some("true");
    Ok(
        json!({"model":MODEL,"revision":REVISION,"dimensions":cfg.embeddings.dimensions,"pipeline":pipeline(&cfg.embeddings),
        "threads_configured":cfg.embeddings.threads,"threads_effective":cfg.embeddings.effective_threads(),
        "runtime_available":cfg!(feature="embeddings"),"candidate_limit_per_branch":100,"index_generation":db.meta_get("embedding_generation")?.and_then(|s|s.parse::<u64>().ok()).unwrap_or(0),
        "indexed_chunks":valid,"invalid_chunks":invalid_chunks,"blocked_files":blocked,"pending_index_files":pending.saturating_sub(blocked),"total_files":files.len(),"excluded_files":db.all_files()?.len().saturating_sub(files.len()),"pending_files":pending,"complete":complete,
        "freshness":if !verified {"unverified"}else if !complete {"partial"}else{"verified"},
        "source_verified":verified,"configured":db.meta_get("embedding_config")?.is_some()}),
    )
}
pub fn index_with(
    db: &mut CacheDb,
    project: &Path,
    full: bool,
    embedder: &mut dyn Embedder,
) -> Result<Value> {
    index_selected_with(db, project, full, embedder, None)
}
pub(crate) fn index_selected_with(
    db: &mut CacheDb,
    project: &Path,
    full: bool,
    embedder: &mut dyn Embedder,
    selection: Option<&crate::selection::Selection>,
) -> Result<Value> {
    crate::operation::check()?;
    let cfg = ProjectConfig::load(project)?;
    compact::check_profile(&cfg.embeddings)?;
    if cfg.embeddings.batching == crate::config::EmbeddingBatching::File {
        recover_staging(project)?;
    }
    let key = config_key(&cfg.embeddings);
    if let Some(old) = db.meta_get("embedding_config")? {
        ensure!(
            old == key || full,
            "embedding configuration changed; run embeddings sync --full"
        );
    }
    if full {
        db.clear_embedding_index()?;
    }
    db.ensure_embedding_counts()?;
    db.meta_set("embedding_config", &key)?;
    db.meta_set("embedding_complete", "false")?;
    let scheduled = if cfg.embeddings.batching == crate::config::EmbeddingBatching::Length
        && selection.is_none()
    {
        Some(scheduler::index(db, project, full, embedder, &cfg, &key)?)
    } else {
        None
    };
    let mut embedded = 0;
    let mut reused = 0;
    let mut unchanged_files = 0;
    let (mut batches, mut actual_tokens, mut padded_tokens) = (0usize, 0usize, 0usize);
    let files = if scheduled.is_some() {
        Vec::new()
    } else {
        scope(db, &cfg)?
            .into_iter()
            .filter(|f| selection.is_none_or(|s| s.accepts(&f.path)))
            .collect()
    };
    if scheduled.is_none() {
        crate::progress::update(|s| s["files_total"] = json!(files.len()));
    }
    for (file_number, file) in files.into_iter().enumerate() {
        crate::progress::update(|s| {
            s["file"] = json!(file.path);
            s["files_done"] = json!(file_number);
        });
        crate::operation::check()?;
        if !file.parsed_ok {
            crate::operation::warning(format!(
                "embedding skipped parser-error file: {}",
                file.path
            ));
            continue;
        }
        if !full
            && db
                .meta_get(&format!("embedding_file:{}", file.path))?
                .as_deref()
                == Some(format!("{key}:{}", file.content_hash).as_str())
        {
            unchanged_files += 1;
            crate::progress::update(|s| s["unchanged_files"] = json!(unchanged_files));
            continue;
        }
        let bytes = std::fs::read(project.join(&file.path))
            .with_context(|| format!("reading {}", file.path))?;
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == file.content_hash,
            "source changed after sync: {}; retry embeddings sync",
            file.path
        );
        drop(bytes); // These bytes only verify freshness; model input comes from the cache.
        let source = db.source_snapshot(&file)?;
        let symbols = db.symbols_for_file(file.id)?;
        crate::progress::phase("chunking");
        crate::progress::update(|s| s["active_files"] = json!(1));
        let chunks = chunk::profile_chunks(
            &file.path,
            &file.content_hash,
            &source,
            &symbols,
            embedder,
            &key,
            cfg.embeddings.chunk_profile,
        )?;
        let old: HashMap<_, _> = db
            .embedding_chunks(Some(&file.path))?
            .into_iter()
            .filter(|r| r.config_key == key)
            .map(|r| (r.chunk.input_hash, r.vector))
            .collect();
        let mut ready = Vec::new();
        let mut todo = Vec::new();
        for c in chunks {
            if let Some(v) = old.get(&c.input_hash) {
                ready.push((c, v.clone()));
                reused += 1;
                crate::progress::update(|s| s["reused_chunks"] = json!(reused));
            } else {
                todo.push(c);
            }
        }
        for batch in todo.chunks(cfg.embeddings.batch_size) {
            crate::progress::phase("inference");
            crate::operation::check()?;
            let lengths = batch
                .iter()
                .map(|c| embedder.input_tokens(&c.text))
                .collect::<Result<Vec<_>>>()?;
            batches += 1;
            crate::progress::update(|s| s["batch"] = json!(batches));
            actual_tokens += lengths.iter().sum::<usize>();
            padded_tokens += lengths.iter().max().copied().unwrap_or(0) * lengths.len();
            let vectors = embedder.embed(batch.iter().map(|c| c.text.clone()).collect())?;
            ensure!(
                vectors.len() == batch.len(),
                "embedding batch returned wrong item count"
            );
            for (c, v) in batch.iter().zip(vectors) {
                ready.push((c.clone(), normalize(&v, cfg.embeddings.dimensions)?));
                embedded += 1;
                crate::progress::update(|s| s["embedded_chunks"] = json!(embedded));
            }
            crate::progress::update(|s| {
                s["unique_inferences"] = json!(embedded);
                s["actual_tokens"] = json!(actual_tokens);
                s["padded_tokens"] = json!(padded_tokens);
            });
            crate::progress::completed();
        }
        crate::operation::check()?;
        ensure!(
            ProjectConfig::load(project)? == cfg,
            "configuration changed during embedding; retry"
        );
        let current = std::fs::read(project.join(&file.path))?;
        ensure!(
            blake3::hash(&current).to_hex().as_str() == file.content_hash,
            "source changed during embedding: {}; retry",
            file.path
        );
        // All chunks for one source snapshot become visible together.
        ready.sort_by(|a, b| {
            a.0.symbol_key
                .cmp(&b.0.symbol_key)
                .then(a.0.chunk_index.cmp(&b.0.chunk_index))
        });
        crate::progress::phase("commit");
        db.commit_embedding_file(&file.path, &file.content_hash, &key, &ready)?;
        crate::progress::update(|s| {
            s["committed_chunks"] =
                json!(s["committed_chunks"].as_u64().unwrap_or(0) + ready.len() as u64);
            s["files_done"] = json!(file_number + 1);
        });
        crate::progress::completed();
    }
    crate::operation::check()?;
    crate::progress::phase("finalizing");
    let mut s = status(db, project, true)?;
    if s["pending_files"] == 0 {
        db.meta_set("embedding_complete", "true")?;
        s["complete"] = json!(true);
        s["freshness"] = json!("verified");
    } else {
        crate::operation::embedding_partial();
    }
    crate::progress::update(|v| {
        v["files_done"] = v["files_total"].clone();
        v["active_files"] = json!(0);
    });
    crate::progress::index(s.clone());
    let mut result = scheduled.unwrap_or_else(|| json!({"embedded_chunks":embedded,"unique_inferences":embedded,"deduplicated_chunks":0,"reused_chunks":reused,"unchanged_files":unchanged_files,"batching":"file","batches":batches,"actual_tokens":actual_tokens,"padded_tokens":padded_tokens,"batch_capacity":batches*cfg.embeddings.batch_size}));
    result["index"] = s;
    result["containers"] = crate::operation::container_stats();
    let capacity = result["batch_capacity"].as_f64().unwrap_or(0.);
    result["batch_fill_ratio"] = json!(if capacity > 0. {
        result["unique_inferences"].as_f64().unwrap_or(0.) / capacity
    } else {
        0.
    });
    let padded = result["padded_tokens"].as_f64().unwrap_or(0.);
    result["padding_ratio"] = json!(if padded > 0. {
        1. - result["actual_tokens"].as_f64().unwrap_or(0.) / padded
    } else {
        0.
    });
    Ok(result)
}
#[derive(Debug, Clone, Serialize)]
pub struct Match {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub snippet: String,
    pub symbol_key: String,
    pub retrieval: String,
    pub cosine_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auxiliary_cosine: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supporting_locations: Vec<SupportingLocation>,
    pub rrf_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_range: Option<[usize; 2]>,
}
#[derive(Debug, Clone, Serialize)]
pub struct SupportingLocation {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub snippet: String,
    pub relation: String,
    pub confidence: f32,
    pub resolution_mode: String,
}
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> Result<f64> {
    ensure!(
        a.len() == b.len() && b.iter().all(|x| x.is_finite()),
        "invalid cached embedding; rebuild with embeddings sync --full"
    );
    let dot = a
        .iter()
        .zip(b)
        .map(|(a, b)| *a as f64 * *b as f64)
        .sum::<f64>();
    let norm = (a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
        * b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>())
    .sqrt();
    ensure!(norm > 1e-12, "zero cached vector; rebuild embeddings");
    Ok(dot / norm)
}
pub fn semantic_with(
    db: &CacheDb,
    project: &Path,
    args: &crate::cli::SearchArgs,
    embedder: &mut dyn Embedder,
) -> Result<Vec<Match>> {
    semantic_candidates_with(db, project, args, embedder, 100)
}
pub(crate) fn semantic_candidates_with(
    db: &CacheDb,
    project: &Path,
    args: &crate::cli::SearchArgs,
    embedder: &mut dyn Embedder,
    limit: usize,
) -> Result<Vec<Match>> {
    semantic_views_with(db, project, args, embedder, limit, false)
}
pub(crate) fn semantic_views_with(
    db: &CacheDb,
    project: &Path,
    args: &crate::cli::SearchArgs,
    embedder: &mut dyn Embedder,
    limit: usize,
    auxiliary: bool,
) -> Result<Vec<Match>> {
    semantic_context_with(db, project, args, embedder, limit, auxiliary, None)
}
pub(crate) fn semantic_context_with(
    db: &CacheDb,
    project: &Path,
    args: &crate::cli::SearchArgs,
    embedder: &mut dyn Embedder,
    limit: usize,
    auxiliary: bool,
    purpose: Option<(&context::Ranking, usize)>,
) -> Result<Vec<Match>> {
    let cfg = ProjectConfig::load(project)?;
    compact::check_profile(&cfg.embeddings)?;
    let key = config_key(&cfg.embeddings);
    let state = status(db, project, !args.no_sync)?;
    ensure!(
        state["configured"] == true,
        "embedding index missing; run embeddings setup then embeddings sync"
    );
    ensure!(
        db.meta_get("embedding_config")?.as_deref() == Some(key.as_str()),
        "embedding configuration changed; run embeddings sync --full"
    );
    let files: HashMap<_, _> = scope(db, &cfg)?
        .into_iter()
        .filter(|f| f.parsed_ok)
        .map(|f| (f.path, f.content_hash))
        .collect();
    let rows = db
        .embedding_chunks(None)?
        .into_iter()
        .filter(|r| {
            r.valid && r.config_key == key && files.get(&r.chunk.path) == Some(&r.chunk.source_hash)
        })
        .collect::<Vec<_>>();
    ensure!(
        !rows.is_empty(),
        "no valid embedded chunks; run embeddings sync"
    );
    if state["pending_files"].as_u64().unwrap_or(0) > 0 || state["complete"] != true {
        crate::operation::embedding_partial();
        crate::operation::warning(
            "embedding index is partial; run embeddings sync to update pending files",
        );
    }
    let query_text = format!("task: code retrieval | query: {}", args.query);
    if embedder.offsets(&query_text)?.len() > 510 {
        crate::operation::warning(
            "semantic query exceeds the 512-token model context and will be truncated",
        );
    }
    crate::operation::check()?;
    let mut inputs = vec![format!("task: code retrieval | query: {}", args.query)];
    if auxiliary {
        inputs.push(format!("task: search result | query: {}", args.query));
    }
    let expected = inputs.len();
    let mut query = embedder.embed(inputs)?;
    ensure!(
        query.len() == expected,
        "query embedding returned wrong count"
    );
    let auxiliary_query = if auxiliary {
        Some(normalize(&query.pop().unwrap(), cfg.embeddings.dimensions)?)
    } else {
        None
    };
    let query = normalize(&query.remove(0), cfg.embeddings.dimensions)?;
    let selection = crate::selection::Selection::new(project, &args.filters)?;
    let _ranking_timer = crate::operation::time("vector_ranking");
    let mut by_symbol: HashMap<String, Match> = HashMap::new();
    for (i, r) in rows.into_iter().enumerate() {
        if i % 128 == 0 {
            crate::operation::check()?;
        }
        let c = r.chunk;
        if !selection.accepts(&c.path)
            || args.kind.as_ref().is_some_and(|k| k != &c.kind)
            || (args.exported && !c.is_exported)
        {
            continue;
        }
        let score = cosine(&query, &r.vector)?;
        let mut item = Match {
            path: c.path,
            name: c.name,
            kind: c.kind,
            line: c.start_line,
            end_line: c.end_line,
            start_byte: c.start_byte,
            end_byte: c.end_byte,
            snippet: c.snippet,
            symbol_key: c.symbol_key.clone(),
            retrieval: "semantic".into(),
            cosine_score: Some(score),
            relevance_score: None,
            auxiliary_cosine: auxiliary_query
                .as_ref()
                .map(|q| cosine(q, &r.vector))
                .transpose()?,
            supporting_locations: vec![],
            rrf_score: None,
            representation: c.representation.clone(),
            symbol_range: c.symbol_range,
        };
        let replace = by_symbol.get(&c.symbol_key).is_none_or(|old| {
            score > old.cosine_score.unwrap()
                || (score == old.cosine_score.unwrap() && item.start_byte < old.start_byte)
        });
        if let Some(old) = by_symbol.get(&c.symbol_key) {
            item.auxiliary_cosine = match (old.auxiliary_cosine, item.auxiliary_cosine) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }
        if replace {
            by_symbol.insert(c.symbol_key, item);
        } else if let Some(old) = by_symbol.get_mut(&c.symbol_key) {
            old.auxiliary_cosine = item.auxiliary_cosine;
        }
    }
    let mut hits: Vec<_> = by_symbol.into_values().collect();
    if let Some((ranking, budget)) = purpose {
        let contexts = context::valid_vectors(db, project, budget)?;
        for hit in &mut hits {
            let (primary, secondary) = if let Some(vector) = contexts.get(&hit.symbol_key) {
                (
                    Some(cosine(&query, vector)?),
                    auxiliary_query
                        .as_ref()
                        .map(|q| cosine(q, vector))
                        .transpose()?,
                )
            } else {
                (None, None)
            };
            hit.relevance_score = ranking.score(
                &hit.kind,
                [hit.cosine_score, hit.auxiliary_cosine, primary, secondary],
            );
        }
    }
    hits.sort_by(|a, b| {
        b.relevance_score
            .or(b.cosine_score)
            .unwrap()
            .total_cmp(&a.relevance_score.or(a.cosine_score).unwrap())
            .then_with(|| tie(a, b))
    });
    crate::operation::omitted_for(hits.len().saturating_sub(limit), "candidate_limit");
    hits.truncate(limit);
    Ok(hits)
}
fn tie(a: &Match, b: &Match) -> std::cmp::Ordering {
    a.path
        .cmp(&b.path)
        .then(a.start_byte.cmp(&b.start_byte))
        .then(a.symbol_key.cmp(&b.symbol_key))
}
pub fn fuse(semantic: Vec<Match>, lexical: Vec<Match>) -> Vec<Match> {
    let _timer = crate::operation::time("fusion");
    let mut combined: HashMap<String, Match> = HashMap::new();
    for branch in [semantic, lexical] {
        for (rank, mut hit) in branch.into_iter().take(100).enumerate() {
            let weight = 1.0 / (60.0 + rank as f64 + 1.0);
            if let Some(old) = combined.get_mut(&hit.symbol_key) {
                old.rrf_score = Some(old.rrf_score.unwrap() + weight);
                old.retrieval = "hybrid".into();
            } else {
                hit.rrf_score = Some(weight);
                combined.insert(hit.symbol_key.clone(), hit);
            }
        }
    }
    let mut hits: Vec<_> = combined.into_values().collect();
    hits.sort_by(|a, b| {
        b.rrf_score
            .unwrap()
            .total_cmp(&a.rrf_score.unwrap())
            .then_with(|| tie(a, b))
    });
    hits
}
pub fn index_prepared(db: &mut CacheDb, project: &Path, full: bool) -> Result<Value> {
    #[cfg(feature = "embeddings")]
    if !full {
        let cfg = ProjectConfig::load(project)?;
        let state = status(db, project, true)?;
        if state["pending_index_files"] == 0
            && db.meta_get("embedding_config")?.as_deref()
                == Some(config_key(&cfg.embeddings).as_str())
        {
            // Verification is still mandatory; a no-op never downloads or conceals
            // missing/corrupt models. No native session is needed without inference.
            model::verify_configured(project)?;
            struct NoInference;
            impl Embedder for NoInference {
                fn offsets(&self, _: &str) -> Result<Vec<(usize, usize)>> {
                    anyhow::bail!("index work changed during verification; retry embeddings sync")
                }
                fn embed(&mut self, _: Vec<String>) -> Result<Vec<Vec<f32>>> {
                    anyhow::bail!("index work changed during verification; retry embeddings sync")
                }
            }
            return index_with(db, project, false, &mut NoInference);
        }
    }
    with_model(project, |m| index_with(db, project, full, m))
}
pub fn sync(project: &Path, full: bool) -> Result<Value> {
    let mut db = CacheDb::open(project)?;
    crate::progress::phase("source_sync");
    crate::service::auto_sync(&mut db, project)?;
    crate::progress::index(status(&db, project, true)?);
    index_prepared(&mut db, project, full)
}
#[cfg(feature = "embeddings")]
pub fn with_model<T>(project: &Path, f: impl FnOnce(&mut dyn Embedder) -> Result<T>) -> Result<T> {
    model::with_model(project, f)
}
#[cfg(not(feature = "embeddings"))]
pub fn with_model<T>(
    _project: &Path,
    _f: impl FnOnce(&mut dyn Embedder) -> Result<T>,
) -> Result<T> {
    anyhow::bail!(
        "this build has no embedding runtime; install the standard package or build with --features embeddings"
    )
}
pub fn run(args: crate::cli::EmbeddingsArgs) -> Result<()> {
    use crate::cli::EmbeddingsCommand::*;
    let (project, json_output, data, unverified) = match args.command {
        Setup {
            project,
            json,
            model_dir,
        } => {
            let p = project.canonicalize()?;
            let data = setup(&p, model_dir.as_deref())?;
            (p, json, data, true)
        }
        Sync {
            project,
            json,
            full,
            progress,
        } => {
            let p = project.canonicalize()?;
            crate::progress::install_interrupt()?;
            let mut guard = match crate::progress::Guard::start(&p, Some(&progress)) {
                Ok(guard) => guard,
                Err(error) => {
                    if json {
                        println!(
                            "{}",
                            json!({"schema_version":1,"status":"error","data":null,"metadata":{"project":p,"operation":null,"warnings":[format!("{error:#}")]}})
                        );
                    }
                    return Err(error);
                }
            };
            let result = sync(&p, full);
            if let Err(error) = &result {
                crate::progress::update(|v| v["error"] = json!(format!("{error:#}")));
            }
            let operation = guard.finish(if crate::operation::cancelled() {
                "cancelled"
            } else if result.is_err() {
                "failed"
            } else {
                "completed"
            })?;
            let mut data = match result {
                Ok(data) => data,
                Err(error) => {
                    if json {
                        println!(
                            "{}",
                            json!({"schema_version":1,"status":"error","data":null,"metadata":{"operation":operation,"warnings":[format!("{error:#}")]}})
                        );
                    }
                    return Err(error);
                }
            };
            data["operation"] = operation;
            (p, json, data, false)
        }
        Status {
            project,
            json,
            files,
            path,
        } => {
            let p = project.canonicalize()?;
            let operation = crate::progress::read(&p)?;
            let db = match CacheDb::open(&p) {
                Ok(db) => db,
                Err(error) => {
                    if let Some(op) =
                        operation.filter(|_| error.to_string().contains("is in use by another"))
                    {
                        let mut data = op["last_index"].clone();
                        if !data.is_object() {
                            data = json!({});
                        }
                        data["freshness"] = json!("unverified");
                        data["source_verified"] = json!(false);
                        data["operation"] = op;
                        data["database_busy"] = json!(true);
                        if files {
                            data["files_available"] = json!(false);
                        }
                        if let Some(path) = path.as_deref() {
                            data["path_scope"] = crate::health::inspect_path(&p, path)?;
                        }
                        let envelope = json!({"schema_version":1,"status":"partial","data":data,"metadata":{"project":p,"freshness":"unverified","warnings":[format!("{error:#}")]}});
                        if json {
                            println!("{envelope}");
                        } else {
                            println!("{}", serde_json::to_string_pretty(&envelope["data"])?);
                        }
                        return Ok(());
                    }
                    return Err(error);
                }
            };
            let mut data = status(&db, &p, false)?;
            data["operation"] = json!(operation);
            if files {
                data["files"] = crate::health::files(&db, &p)?;
            }
            if let Some(path) = path {
                data["path_scope"] = crate::health::inspect_path(&p, &path)?;
            }
            (p, json, data, true)
        }
    };
    if json_output {
        println!("{}", crate::output::json(&project, unverified, data)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&data)?);
    }
    Ok(())
}
#[cfg(feature = "embeddings")]
fn setup(project: &Path, model_dir: Option<&Path>) -> Result<Value> {
    model::setup(project, model_dir)
}
#[cfg(not(feature = "embeddings"))]
fn setup(_project: &Path, _model_dir: Option<&Path>) -> Result<Value> {
    anyhow::bail!(
        "this build has no embedding runtime; install a standard embedding-enabled package"
    )
}
