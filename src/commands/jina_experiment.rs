//! Explicit CPU experiment. No production model selection or index is changed.
use crate::embeddings::{self as embedding, Chunk, Embedder, Match};
use anyhow::{Context, Result, ensure};
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const REVISION: &str = "516f4baf13dec4ddddda8631e019b5737c8bc250";
struct Jina {
    model: TextEmbedding,
    tokenizer: tokenizers::Tokenizer,
}
impl Embedder for Jina {
    fn offsets(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        Ok(self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .get_offsets()
            .iter()
            .copied()
            .filter(|(a, b)| b > a)
            .collect())
    }
    fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let batch = texts.len();
        Ok(self.model.embed(texts, Some(batch))?)
    }
}
struct Indexed {
    chunk: Chunk,
    vector: Vec<f32>,
}
fn save(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn tokenizer(path: &Path) -> Result<tokenizers::Tokenizer> {
    let mut t =
        tokenizers::Tokenizer::from_file(path).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    t.with_truncation(None)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    t.with_padding(None);
    Ok(t)
}
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot = a
        .iter()
        .zip(b)
        .map(|(x, y)| *x as f64 * *y as f64)
        .sum::<f64>();
    dot / (a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
        * b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>())
    .sqrt()
}
fn ranked(rows: &[Indexed], query: &[f32]) -> Vec<Match> {
    let mut symbols: HashMap<&str, Match> = HashMap::new();
    for row in rows {
        let c = &row.chunk;
        let score = cosine(query, &row.vector);
        let replace = symbols.get(c.symbol_key.as_str()).is_none_or(|old| {
            score > old.cosine_score.unwrap()
                || score == old.cosine_score.unwrap() && c.start_byte < old.start_byte
        });
        if replace {
            symbols.insert(
                &c.symbol_key,
                Match {
                    semantic: None,
                    path: c.path.clone(),
                    name: c.name.clone(),
                    kind: c.kind.clone(),
                    line: c.start_line,
                    end_line: c.end_line,
                    start_byte: c.start_byte,
                    end_byte: c.end_byte,
                    snippet: c.snippet.clone(),
                    symbol_key: c.symbol_key.clone(),
                    retrieval: "semantic".into(),
                    cosine_score: Some(score),
                    relevance_score: None,
                    auxiliary_cosine: None,
                    supporting_locations: vec![],
                    rrf_score: None,
                    representation: c.representation.clone(),
                    symbol_range: c.symbol_range,
                },
            );
        }
    }
    let mut hits: Vec<_> = symbols.into_values().collect();
    hits.sort_by(|a, b| {
        b.cosine_score
            .unwrap()
            .total_cmp(&a.cosine_score.unwrap())
            .then(a.path.cmp(&b.path))
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.symbol_key.cmp(&b.symbol_key))
    });
    hits.truncate(100);
    hits
}
fn lexical(db: &crate::cache::CacheDb, project: &Path, query: &str) -> Result<Vec<Match>> {
    use clap::Parser;
    let crate::cli::Command::Search(args) = crate::cli::Cli::try_parse_from([
        "code-rcl",
        "search",
        query,
        "--hybrid",
        "--no-sync",
        "--project",
        project.to_str().unwrap(),
    ])?
    .command
    else {
        unreachable!()
    };
    let selection = crate::selection::Selection::new(project, &args.filters)?;
    let rows = super::hybrid_lexical_candidates(db, &args, &selection)?;
    let mut hits = Vec::new();
    for (s, path) in rows.into_iter().take(100) {
        let source = fs::read_to_string(project.join(&path))?;
        let start = s.start_byte.unwrap_or(0).max(0) as usize;
        let end = (s.end_byte.unwrap_or(0).max(0) as usize).min(source.len());
        hits.push(Match {
            semantic: None,
            symbol_key: embedding::chunk::symbol_key(&path, &s),
            path,
            name: s.name,
            kind: s.kind.to_string(),
            line: s.start_line.unwrap_or(1) as usize,
            end_line: s.end_line.unwrap_or(1) as usize,
            start_byte: start,
            end_byte: end,
            snippet: source
                .get(start..end)
                .unwrap_or("")
                .chars()
                .take(1600)
                .collect(),
            retrieval: "lexical".into(),
            cosine_score: None,
            relevance_score: None,
            auxiliary_cosine: None,
            supporting_locations: vec![],
            rrf_score: None,
            representation: None,
            symbol_range: None,
        });
    }
    Ok(hits)
}

#[test]
#[ignore = "explicit pinned Jina CPU comparison; set CODE_RCL_JINA_* variables"]
fn jina_cpu_experiment() -> Result<()> {
    crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let project = PathBuf::from(std::env::var("CODE_RCL_JINA_PROJECT")?);
    let output = PathBuf::from(std::env::var("CODE_RCL_JINA_OUTPUT")?);
    ensure!(
        !output.exists(),
        "output exists; use a fresh experiment directory"
    );
    fs::create_dir_all(&output)?;
    let model_dir = PathBuf::from(std::env::var("CODE_RCL_JINA_MODEL_DIR")?);
    let gemma_dir = PathBuf::from(std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR")?);
    let variant = std::env::var("CODE_RCL_JINA_VARIANT")?;
    ensure!(
        ["gemma", "fp32", "int8"].contains(&variant.as_str()),
        "unknown variant"
    );
    let manifest: Value = serde_json::from_slice(&fs::read(model_dir.join("manifest.json"))?)?;
    ensure!(
        manifest["revision"] == REVISION,
        "unexpected model revision"
    );
    for file in manifest["files"].as_array().context("manifest files")? {
        let p = model_dir.join(file["file"].as_str().unwrap());
        ensure!(
            fs::metadata(&p)?.len() == file["bytes"].as_u64().unwrap()
                && hash(&p)? == file["sha256"].as_str().unwrap(),
            "artifact checksum mismatch: {}",
            p.display()
        );
    }
    let mut db = crate::cache::CacheDb::open(&project)?;
    let stats = crate::service::auto_sync(&mut db, &project)?;
    ensure!(
        stats.changed == 0 && stats.added == 0 && stats.removed == 0,
        "frozen source changed"
    );
    let baseline = db.embedding_chunks(None)?;
    ensure!(
        baseline.len() == 4030 && baseline.iter().all(|r| r.valid),
        "expected complete Gemma baseline"
    );
    let gemma_tokens = tokenizer(&gemma_dir.join("tokenizer.json"))?;
    let jina_tokens = tokenizer(&model_dir.join("tokenizer.json"))?;
    let mut calibration = Vec::new();
    for extension in ["rs", "py", "go", "kt"] {
        let mut rows: Vec<_> = baseline
            .iter()
            .filter(|r| {
                Path::new(&r.chunk.path)
                    .extension()
                    .is_some_and(|e| e == extension)
            })
            .collect();
        rows.sort_by(|a, b| a.chunk.input_hash.cmp(&b.chunk.input_hash));
        ensure!(rows.len() >= 24, "insufficient language sample");
        for n in 0..24 {
            let mut text: String = rows[n * (rows.len() - 1) / 23]
                .chunk
                .text
                .chars()
                .take(384)
                .collect();
            loop {
                let count = |t: &tokenizers::Tokenizer| {
                    t.encode(text.as_str(), false)
                        .map(|e| e.len())
                        .map_err(|e| anyhow::anyhow!(e.to_string()))
                };
                if count(&gemma_tokens)? <= 510 && count(&jina_tokens)? <= 510 {
                    break;
                }
                ensure!(text.pop().is_some(), "empty calibration text");
            }
            calibration.push(text);
        }
    }
    save(&output.join("calibration-inputs.json"), &json!(calibration))?;
    drop(gemma_tokens);
    drop(jina_tokens);
    if variant == "gemma" {
        embedding::with_model(&project, |model| {
            run(
                &db,
                &project,
                &output,
                &variant,
                model,
                Some(baseline),
                &calibration,
            )
        })
    } else {
        drop(baseline);
        let start = Instant::now();
        let filename = if variant == "fp32" {
            "onnx/model.onnx"
        } else {
            "onnx/model_quantized.onnx"
        };
        let files = TokenizerFiles {
            tokenizer_file: fs::read(model_dir.join("tokenizer.json"))?,
            config_file: fs::read(model_dir.join("config.json"))?,
            special_tokens_map_file: fs::read(model_dir.join("special_tokens_map.json"))?,
            tokenizer_config_file: fs::read(model_dir.join("tokenizer_config.json"))?,
        };
        let mut definition =
            UserDefinedEmbeddingModel::new(fs::read(model_dir.join(filename))?, files)
                .with_pooling(Pooling::Mean);
        if variant == "int8" {
            definition = definition.with_quantization(QuantizationMode::Dynamic);
        }
        let model = TextEmbedding::try_new_from_user_defined(
            definition,
            InitOptionsUserDefined::new()
                .with_max_length(512)
                .with_intra_threads(4),
        )?;
        save(
            &output.join("runtime.json"),
            &json!({"variant":variant,"load_ms":start.elapsed().as_secs_f64()*1000.,"model_revision":REVISION,"onnx_file":filename,"threads":4,"max_length":512,"pooling":"mean","runtime":"FastEmbed Rust 7.5.0 + ONNX Runtime CPU; no Python inference"}),
        )?;
        let mut model = Jina {
            tokenizer: tokenizer(&model_dir.join("tokenizer.json"))?,
            model,
        };
        run(
            &db,
            &project,
            &output,
            &variant,
            &mut model,
            None,
            &calibration,
        )
    }
}

fn run(
    db: &crate::cache::CacheDb,
    project: &Path,
    out: &Path,
    variant: &str,
    model: &mut dyn Embedder,
    baseline: Option<Vec<crate::cache::embeddings::StoredChunk>>,
    calibration: &[String],
) -> Result<()> {
    let mut report = json!({"variant":variant,"threads":4,"batch_size":4,"max_length":512,"dimensions":768,"query_prefix":if variant=="gemma" {"task: code retrieval | query: "} else {""},"vectors_are_separate_from_production_cache":true});
    model.embed(calibration[..4].to_vec())?;
    let mut calibration_ms = Vec::new();
    for pass in 0..3 {
        let started = Instant::now();
        for batch in calibration.chunks(4) {
            let values = model.embed(batch.to_vec())?;
            ensure!(values.len() == batch.len(), "calibration count");
        }
        calibration_ms.push(started.elapsed().as_secs_f64() * 1000.);
        eprintln!(
            "CALIBRATION {variant} pass={} ms={:.0}",
            pass + 1,
            calibration_ms.last().unwrap()
        );
    }
    report["shared_96_input_inference_ms"] = json!(calibration_ms);
    report["shared_96_input_token_count"] = json!(
        calibration
            .iter()
            .map(|text| model.input_tokens(text))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum::<usize>()
    );
    report["crate_version"] = json!(env!("CARGO_PKG_VERSION"));
    let batched = model.embed(calibration[..8].to_vec())?;
    let mut minimum = 1f64;
    for (text, vector) in calibration[..8].iter().zip(&batched) {
        let single = model.embed(vec![text.clone()])?;
        minimum = minimum.min(cosine(vector, &single[0]));
    }
    report["batch_vs_single_min_cosine"] = json!(minimum);
    save(&out.join("measurements.json"), &report)?;
    let mut rows = Vec::new();
    if let Some(baseline) = baseline {
        rows = baseline
            .into_iter()
            .map(|r| Indexed {
                chunk: r.chunk,
                vector: r.vector,
            })
            .collect();
        report["indexing"] =
            json!({"source":"existing Gemma index; not cold indexing","chunks":rows.len()});
    } else {
        let cfg = crate::config::ProjectConfig::load(project)?;
        let files = embedding::scope(db, &cfg)?;
        let mut vectors = BufWriter::new(fs::File::create(out.join("vectors.f32"))?);
        let mut inputs = BufWriter::new(fs::File::create(out.join("inputs.jsonl"))?);
        let start = Instant::now();
        let mut inference_ms = 0f64;
        let mut actual_tokens = 0usize;
        let mut padded_tokens = 0usize;
        for (number, file) in files.iter().enumerate() {
            ensure!(file.parsed_ok, "parser failure in frozen corpus");
            let source = db.source_snapshot(file)?;
            let symbols = db.symbols_for_file(file.id)?;
            let key = format!(
                "experimental:jina-code-v2:{REVISION}:{variant}:shared-header-full-512-o64"
            );
            let chunks = embedding::chunk::profile_chunks(
                &file.path,
                &file.content_hash,
                &source,
                &symbols,
                model,
                &key,
                crate::config::EmbeddingChunkProfile::Full,
            )?;
            for batch in chunks.chunks(4) {
                let lengths = batch
                    .iter()
                    .map(|c| model.input_tokens(&c.text))
                    .collect::<Result<Vec<_>>>()?;
                actual_tokens += lengths.iter().sum::<usize>();
                padded_tokens += lengths.iter().max().copied().unwrap_or(0) * batch.len();
                let tick = Instant::now();
                let output = model.embed(batch.iter().map(|c| c.text.clone()).collect())?;
                inference_ms += tick.elapsed().as_secs_f64() * 1000.;
                ensure!(output.len() == batch.len(), "index vector count");
                for (chunk, vector) in batch.iter().zip(output) {
                    let vector = embedding::normalize(&vector, 768)?;
                    for value in &vector {
                        vectors.write_all(&value.to_le_bytes())?;
                    }
                    writeln!(
                        inputs,
                        "{}",
                        json!({"symbol_key":chunk.symbol_key,"chunk_index":chunk.chunk_index,"text_hash":blake3::hash(chunk.text.as_bytes()).to_hex().to_string(),"input_hash":chunk.input_hash,"source_hash":chunk.source_hash})
                    )?;
                    rows.push(Indexed {
                        chunk: chunk.clone(),
                        vector,
                    });
                }
            }
            vectors.flush()?;
            inputs.flush()?;
            let progress = json!({"files_done":number+1,"files_total":files.len(),"chunks":rows.len(),"actual_tokens":actual_tokens,"padded_tokens":padded_tokens,"inference_ms":inference_ms,"elapsed_ms":start.elapsed().as_secs_f64()*1000.});
            save(&out.join("index-progress.json"), &progress)?;
            eprintln!(
                "JINA {variant} files={}/{} chunks={} inference_ms={inference_ms:.0}",
                number + 1,
                files.len(),
                rows.len()
            );
        }
        report["indexing"] = json!({"source":"cold Jina index in isolated experiment","chunks":rows.len(),"actual_tokens":actual_tokens,"padded_tokens":padded_tokens,"inference_ms":inference_ms,"elapsed_ms":start.elapsed().as_secs_f64()*1000.});
        report["indexing"]["runtime_state"] =
            json!("model already loaded and warmed; model load and source parsing excluded");
    }
    save(&out.join("measurements.json"), &report)?;
    let mut queries: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/retrieval_bkndb/queries.json"
    ))?;
    let extra: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/retrieval_bkndb/container-queries.json"
    ))?;
    queries["queries"]
        .as_array_mut()
        .unwrap()
        .extend(extra["queries"].as_array().unwrap().iter().cloned());
    let mut rankings = BufWriter::new(fs::File::create(out.join("rankings.jsonl"))?);
    let mut latency = Vec::new();
    for pass in 0..3 {
        for query in queries["queries"].as_array().unwrap() {
            let text = query["query"].as_str().unwrap();
            let input = if variant == "gemma" {
                format!("task: code retrieval | query: {text}")
            } else {
                text.into()
            };
            let start = Instant::now();
            let vector = model.embed(vec![input])?;
            let query_ms = start.elapsed().as_secs_f64() * 1000.;
            let vector = embedding::normalize(&vector[0], 768)?;
            latency.push(json!({"pass":pass,"query":query["id"],"query_inference_ms":query_ms}));
            if pass == 0 {
                let tick = Instant::now();
                let semantic = ranked(&rows, &vector);
                let semantic_rank_ms = tick.elapsed().as_secs_f64() * 1000.;
                let lexical = lexical(db, project, text)?;
                let hybrid = embedding::fuse(semantic.clone(), lexical);
                for (mode, hits) in [
                    ("semantic", semantic),
                    ("hybrid", hybrid.into_iter().take(100).collect()),
                ] {
                    writeln!(
                        rankings,
                        "{}",
                        json!({"query":query,"mode":mode,"ranking":hits,"query_inference_ms":query_ms,"semantic_rank_ms":semantic_rank_ms})
                    )?;
                }
                rankings.flush()?;
            }
        }
        eprintln!("QUERY PASS {variant} pass={} completed=112", pass + 1);
    }
    report["query_latencies"] = json!(latency);
    report["completed_queries"] = json!(112);
    save(&out.join("measurements.json"), &report)?;
    eprintln!("COMPLETED JINA EXPERIMENT {variant}");
    Ok(())
}
