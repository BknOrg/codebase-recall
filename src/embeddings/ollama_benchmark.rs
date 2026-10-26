//! Isolated native benchmark only; does not register a production provider.
use super::*;
use std::time::Duration;

struct Ollama {
    tokenizer: tokenizers::Tokenizer,
    agent: ureq::Agent,
    model: String,
    server_total_ns: u64,
    server_load_ns: u64,
    server_tokens: u64,
    calls: usize,
}
impl Embedder for Ollama {
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
    fn input_tokens(&self, text: &str) -> Result<usize> {
        Ok(self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .len())
    }
    fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let count = texts.len();
        let body = json!({"model":self.model,"input":texts,"truncate":false,"keep_alive":"10m",
            "options":{"num_ctx":2048}})
        .to_string();
        let raw = self
            .agent
            .post("http://127.0.0.1:11434/api/embed")
            .header("Content-Type", "application/json")
            .send(body.as_str())?
            .body_mut()
            .read_to_string()?;
        let response: Value = serde_json::from_str(&raw)?;
        ensure!(
            response.get("error").is_none(),
            "Ollama returned error: {response}"
        );
        let vectors: Vec<Vec<f32>> = serde_json::from_value(response["embeddings"].clone())?;
        ensure!(
            vectors.len() == count,
            "Ollama returned incorrect vector count"
        );
        self.server_total_ns += response["total_duration"].as_u64().unwrap_or(0);
        self.server_load_ns += response["load_duration"].as_u64().unwrap_or(0);
        self.server_tokens += response["prompt_eval_count"].as_u64().unwrap_or(0);
        self.calls += 1;
        Ok(vectors)
    }
}

fn ollama_digest(agent: &ureq::Agent, model: &str) -> Result<String> {
    let raw = agent
        .get("http://127.0.0.1:11434/api/tags")
        .call()?
        .body_mut()
        .read_to_string()?;
    let tags: Value = serde_json::from_str(&raw)?;
    tags["models"]
        .as_array()
        .context("missing Ollama models")?
        .iter()
        .find(|m| m["name"] == model || m["model"] == model)
        .and_then(|m| m["digest"].as_str())
        .map(str::to_owned)
        .context("Ollama model missing")
}

pub(super) fn run(output: &Path, model_dir: &str) -> Result<()> {
    let variant = std::env::var("CODE_RCL_OLLAMA_VARIANT")?;
    ensure!(
        ["ollama", "fastembed"].contains(&variant.as_str()),
        "invalid experiment variant"
    );
    let source_path = PathBuf::from(std::env::var("CODE_RCL_BENCHMARK_CORPUS")?);
    let raw = fs::read(&source_path)?;
    let corpus: Value = serde_json::from_slice(&raw)?;
    let manifest: Value = serde_json::from_slice(&fs::read(format!(
        "{}.manifest.json",
        source_path.display()
    ))?)?;
    ensure!(
        format!("{:x}", Sha256::digest(&raw)) == manifest["corpus_sha256"],
        "source manifest mismatch"
    );
    let project = output.join("workspace");
    materialize(&project, &corpus, &manifest)?;
    config(&project, model_dir, 4, 4, "file", "full")?;
    if let Some(sync) = manifest.get("sync") {
        let p = project.join(".code-rcl/config.toml");
        let mut doc: toml::Value = toml::from_str(&fs::read_to_string(&p)?)?;
        set_sync_config(&mut doc, sync)?;
        fs::write(p, toml::to_string(&doc)?)?;
    }
    check_bytes(&project, &corpus)?;
    let cfg = ProjectConfig::load(&project)?;
    let model_name = std::env::var("CODE_RCL_OLLAMA_MODEL")?;
    let expected_digest = std::env::var("CODE_RCL_OLLAMA_DIGEST")?;
    let batch: usize = std::env::var("CODE_RCL_OLLAMA_BATCH")
        .unwrap_or_else(|_| "4".into())
        .parse()?;
    ensure!(
        [1, 2, 4, 8].contains(&batch),
        "invalid Ollama experiment batch"
    );
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .timeout_connect(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    if variant == "ollama" {
        ensure!(
            ollama_digest(&agent, &model_name)? == expected_digest,
            "Ollama model changed"
        );
    }
    let key = if variant == "fastembed" {
        config_key(&cfg.embeddings)
    } else {
        format!(
            "experiment:ollama:{model_name}:{expected_digest}:768:auto-resources-context2048:gemma1-full-t512-o64"
        )
    };
    let total = Instant::now();
    let sync_start = Instant::now();
    let mut db = CacheDb::open(&project)?;
    ensure!(
        db.embedding_chunks(None)?.is_empty(),
        "cold benchmark already contains vectors"
    );
    let stats = crate::service::auto_sync(&mut db, &project)?;
    let source_sync_ms = sync_start.elapsed().as_secs_f64() * 1000.;
    let load_start = Instant::now();
    let mut report = if variant == "fastembed" {
        with_model(&project, |model| {
            let acquire = load_start.elapsed().as_secs_f64() * 1000.;
            let mut value = index(&db, &project, output, &cfg, &key, 4, model)?;
            value["model_acquire_ms"] = json!(acquire);
            Ok(value)
        })?
    } else {
        let mut tokenizer =
            tokenizers::Tokenizer::from_file(Path::new(model_dir).join("tokenizer.json"))
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        tokenizer
            .with_truncation(None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        tokenizer.with_padding(None);
        let mut model = Ollama {
            tokenizer,
            agent,
            model: model_name.clone(),
            server_total_ns: 0,
            server_load_ns: 0,
            server_tokens: 0,
            calls: 0,
        };
        let acquire = load_start.elapsed().as_secs_f64() * 1000.;
        let mut value = index(&db, &project, output, &cfg, &key, batch, &mut model)?;
        value["model_acquire_ms"] = json!(acquire);
        value["ollama_server_total_ms"] = json!(model.server_total_ns as f64 / 1e6);
        value["ollama_server_load_ms"] = json!(model.server_load_ns as f64 / 1e6);
        value["ollama_actual_tokens"] = json!(model.server_tokens);
        value["ollama_calls"] = json!(model.calls);
        ensure!(
            ollama_digest(&model.agent, &model_name)? == expected_digest,
            "Ollama model changed during indexing"
        );
        value
    };
    report["cold_index_wall_ms"] = json!(total.elapsed().as_secs_f64() * 1000.);
    report["source_sync_ms"] = json!(source_sync_ms);
    report["source_files_scanned"] = json!(stats.scanned);
    report["variant"] = json!(variant);
    report["model"] = json!(if variant == "fastembed" {
        MODEL
    } else {
        model_name.as_str()
    });
    report["model_digest"] = json!(if variant == "fastembed" {
        REVISION
    } else {
        expected_digest.as_str()
    });
    report["batch_size"] = json!(if variant == "fastembed" { 4 } else { batch });
    report["config_key"] = json!(key);
    report["corpus_sha256"] = manifest["corpus_sha256"].clone();
    report["binary_sha256"] = json!(format!(
        "{:x}",
        Sha256::digest(fs::read(std::env::current_exe()?)?)
    ));
    let verified = Instant::now();
    let rows = db.embedding_chunks(None)?;
    ensure!(
        rows.len() == report["chunks"].as_u64().unwrap() as usize,
        "database chunk count mismatch"
    );
    ensure!(
        rows.iter()
            .all(|r| r.valid && r.config_key == key && r.vector.len() == 768),
        "invalid database vectors"
    );
    drop(db);
    check_bytes(&project, &corpus)?;
    report["database_readback_ms"] = json!(verified.elapsed().as_secs_f64() * 1000.);
    report["source_bytes_verified"] = json!(true);
    report["database_readback_verified"] = json!(true);
    if std::env::var_os("CODE_RCL_OLLAMA_QUALITY").is_some() {
        let quality = if variant == "fastembed" {
            with_model(&project, quality)?
        } else {
            let mut tokenizer =
                tokenizers::Tokenizer::from_file(Path::new(model_dir).join("tokenizer.json"))
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            tokenizer
                .with_truncation(None)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            tokenizer.with_padding(None);
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(600)))
                .timeout_connect(Some(Duration::from_secs(5)))
                .build()
                .new_agent();
            let mut model = Ollama {
                tokenizer,
                agent,
                model: model_name.clone(),
                server_total_ns: 0,
                server_load_ns: 0,
                server_tokens: 0,
                calls: 0,
            };
            quality(&mut model)?
        };
        save(&output.join("quality32.json"), &quality)?;
    }
    save(&output.join("measurements.json"), &report)?;
    eprintln!("COMPLETED OLLAMA COMPARISON {variant}: {report}");
    Ok(())
}

fn index(
    db: &CacheDb,
    project: &Path,
    output: &Path,
    cfg: &ProjectConfig,
    key: &str,
    batch: usize,
    model: &mut dyn Embedder,
) -> Result<Value> {
    let files = scope(db, cfg)?;
    let mut chunks_count = 0;
    let mut blocked = 0;
    let mut chunking_ms = 0.;
    let mut inference_ms = 0.;
    let mut commit_ms = 0.;
    let mut batches = 0;
    let mut actual_tokens = 0;
    let mut padded_tokens = 0;
    let mut inputs = std::io::BufWriter::new(fs::File::create(output.join("inputs.jsonl"))?);
    let mut vectors = std::io::BufWriter::new(fs::File::create(output.join("vectors.f32"))?);
    let started = Instant::now();
    for (n, file) in files.iter().enumerate() {
        if !file.parsed_ok {
            blocked += 1;
            continue;
        }
        let tick = Instant::now();
        ensure!(
            blake3::hash(&fs::read(project.join(&file.path))?)
                .to_hex()
                .as_str()
                == file.content_hash,
            "source changed"
        );
        let source = db.source_snapshot(file)?;
        let symbols = db.symbols_for_file(file.id)?;
        let chunks = chunk::profile_chunks(
            &file.path,
            &file.content_hash,
            &source,
            &symbols,
            model,
            key,
            crate::config::EmbeddingChunkProfile::Full,
        )?;
        chunking_ms += tick.elapsed().as_secs_f64() * 1000.;
        let mut ready = Vec::new();
        for group in chunks.chunks(batch) {
            let lengths = group
                .iter()
                .map(|c| model.input_tokens(&c.text))
                .collect::<Result<Vec<_>>>()?;
            actual_tokens += lengths.iter().sum::<usize>();
            padded_tokens += lengths.iter().max().copied().unwrap_or(0) * group.len();
            let tick = Instant::now();
            let values = model.embed(group.iter().map(|c| c.text.clone()).collect())?;
            inference_ms += tick.elapsed().as_secs_f64() * 1000.;
            ensure!(values.len() == group.len(), "vector count mismatch");
            for (c, v) in group.iter().zip(values) {
                ready.push((c.clone(), normalize(&v, 768)?));
            }
            batches += 1;
        }
        let tick = Instant::now();
        ensure!(
            blake3::hash(&fs::read(project.join(&file.path))?)
                .to_hex()
                .as_str()
                == file.content_hash,
            "source changed before commit"
        );
        db.commit_embedding_file(&file.path, &file.content_hash, key, &ready)?;
        commit_ms += tick.elapsed().as_secs_f64() * 1000.;
        for (c, v) in ready {
            writeln!(
                inputs,
                "{}",
                json!({"path":c.path,"symbol_key":c.symbol_key,"chunk_index":c.chunk_index,
                "source_hash":c.source_hash,"text_hash":blake3::hash(c.text.as_bytes()).to_hex().to_string(),
                "canonical_input_hash":blake3::hash(format!("{}\n{}",config_key(&cfg.embeddings),c.text).as_bytes()).to_hex().to_string(),
                "start_byte":c.start_byte,"end_byte":c.end_byte,"start_line":c.start_line,"end_line":c.end_line})
            )?;
            for value in v {
                vectors.write_all(&value.to_le_bytes())?;
            }
            chunks_count += 1;
        }
        inputs.flush()?;
        vectors.flush()?;
        let progress = json!({"files_done":n+1,"files_total":files.len(),"chunks":chunks_count,"inference_ms":inference_ms,"elapsed_ms":started.elapsed().as_secs_f64()*1000.});
        save(&output.join("progress.json"), &progress)?;
        eprintln!(
            "index {}/{} chunks={chunks_count} inference_ms={inference_ms:.0}",
            n + 1,
            files.len()
        );
    }
    db.meta_set("embedding_config", key)?;
    Ok(
        json!({"files":files.len(),"blocked_files":blocked,"chunks":chunks_count,"chunking_ms":chunking_ms,
        "inference_ms":inference_ms,"commit_ms":commit_ms,"batches":batches,"gemma1_input_tokens":actual_tokens,
        "gemma1_padded_tokens":padded_tokens,"index_stage_ms":started.elapsed().as_secs_f64()*1000.,"reuse":0}),
    )
}

fn quality(model: &mut dyn Embedder) -> Result<Value> {
    let data: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/embedding_accuracy/dataset.json"
    ))?;
    let docs = data["documents"].as_array().unwrap();
    let queries = data["queries"].as_array().unwrap();
    let mut vectors = Vec::new();
    for group in docs.chunks(4) {
        vectors.extend(
            model.embed(
                group
                    .iter()
                    .map(|d| {
                        format!(
                            "title: {} | text: {}",
                            d["path"].as_str().unwrap(),
                            d["text"].as_str().unwrap()
                        )
                    })
                    .collect(),
            )?,
        );
    }
    ensure!(vectors.len() == docs.len(), "quality document vector count");
    let mut records = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        let values = model.embed(vec![format!(
            "task: code retrieval | query: {}",
            q["text"].as_str().unwrap()
        )])?;
        ensure!(values.len() == 1, "quality query vector count");
        for dim in [768, 256] {
            let query = normalize(&values[0], dim)?;
            let mut scores = vectors
                .iter()
                .enumerate()
                .map(|(n, v)| Ok((n, cosine(&query, &normalize(v, dim)?)?)))
                .collect::<Result<Vec<_>>>()?;
            scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let rank = scores
                .iter()
                .position(|(n, _)| *n == q["gold"].as_u64().unwrap() as usize)
                .unwrap()
                + 1;
            records.push(json!({"query":i,"text":q["text"],"dimensions":dim,"rank":rank,"hit1":usize::from(rank==1),
                "hit5":usize::from(rank<=5),"mrr":1./rank as f64,"ndcg5":if rank<=5 {1./((rank+1)as f64).log2()} else {0.}}));
        }
    }
    Ok(
        json!({"documents":docs.len(),"queries":queries.len(),"scope":"legacy seed fixture; not general retrieval quality; no BknDb112 query evaluation", "records":records}),
    )
}
