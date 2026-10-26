//! Explicit, retained-workspace model harness; never touches the user's index.
use super::*;
#[path = "first_index_benchmark.rs"]
mod first_index;
#[path = "ollama_benchmark.rs"]
mod ollama_benchmark;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, time::Instant};
fn save(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn config(
    project: &Path,
    model: &str,
    threads: usize,
    batch: usize,
    strategy: &str,
    profile: &str,
) -> Result<()> {
    fs::create_dir_all(project.join(".code-rcl"))?;
    fs::write(
        project.join(".code-rcl/config.toml"),
        format!(
            "[embeddings]\nmodel_dir={}\nthreads={threads}\nbatch_size={batch}\nbatching='{strategy}'\nchunk_profile='{profile}'\n",
            serde_json::to_string(model)?
        ),
    )?;
    Ok(())
}
fn set_sync_config(doc: &mut toml::Value, sync: &Value) -> Result<()> {
    doc.as_table_mut()
        .context("benchmark config table")?
        .insert("sync".into(), toml::Value::try_from(sync)?);
    Ok(())
}
#[test]
fn benchmark_sync_config_inserts_missing_table_and_preserves_embeddings() -> Result<()> {
    let mut doc: toml::Value = toml::from_str("[embeddings]\nthreads=4\n")?;
    set_sync_config(&mut doc, &json!({"max_file_kb":512,"languages":["rust"]}))?;
    assert_eq!(doc["sync"]["max_file_kb"].as_integer(), Some(512));
    assert_eq!(doc["embeddings"]["threads"].as_integer(), Some(4));
    set_sync_config(&mut doc, &json!({"max_file_kb":1024,"languages":[]}))?;
    assert_eq!(doc["sync"]["max_file_kb"].as_integer(), Some(1024));
    Ok(())
}
fn corpus() -> Result<(Value, Value)> {
    let raw = include_bytes!("../../tests/fixtures/retrieval_bkndb/corpus.json");
    let manifest: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/retrieval_bkndb/manifest.json"
    ))?;
    ensure!(
        format!("{:x}", Sha256::digest(raw)) == manifest["corpus_sha256"],
        "corpus fingerprint mismatch"
    );
    Ok((serde_json::from_slice(raw)?, manifest))
}
fn materialize(project: &Path, corpus: &Value, manifest: &Value) -> Result<()> {
    ensure!(
        !project.exists(),
        "benchmark workspace already exists; use a new output directory"
    );
    fs::create_dir_all(project)?;
    for (path, source) in corpus.as_object().context("corpus object")? {
        let bytes = source.as_str().context("source text")?.as_bytes();
        ensure!(
            format!("{:x}", Sha256::digest(bytes)) == manifest["files"][path],
            "source fingerprint mismatch"
        );
        let relative = Path::new(path);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "unsafe corpus path"
        );
        let target = project.join(relative);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(target, bytes)?;
    }
    Ok(())
}
fn check_bytes(project: &Path, corpus: &Value) -> Result<()> {
    for (path, source) in corpus.as_object().unwrap() {
        ensure!(
            fs::read(project.join(path))? == source.as_str().unwrap().as_bytes(),
            "source bytes changed: {path}"
        );
    }
    Ok(())
}
#[test]
#[ignore = "explicit real-model retained-workspace calibration/corpus benchmark"]
fn embedding_index_benchmark() -> Result<()> {
    crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let output = PathBuf::from(
        std::env::var("CODE_RCL_BENCHMARK_OUTPUT").context("set benchmark output directory")?,
    );
    fs::create_dir_all(&output)?;
    let model = std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR")?;
    let mode = std::env::var("CODE_RCL_BENCHMARK_MODE").unwrap_or_else(|_| "calibration".into());
    let strategy = std::env::var("CODE_RCL_BENCHMARK_BATCHING").unwrap_or_else(|_| "file".into());
    let profile = std::env::var("CODE_RCL_BENCHMARK_PROFILE").unwrap_or_else(|_| "full".into());
    if mode == "legacy32" {
        let project = output.join("workspace");
        ensure!(!project.exists(), "benchmark workspace exists");
        config(&project, &model, 4, 4, "file", "full")?;
        return legacy_quality(&project, &output);
    }
    if mode == "ollama-index" {
        return ollama_benchmark::run(&output, &model);
    }
    if mode == "snapshot" {
        return first_index::snapshot(&output);
    }
    if mode == "calibration-case" {
        return first_index::calibration_case(&output, &model);
    }
    if mode == "strict-legacy" {
        return first_index::legacy(&output, &model, &strategy);
    }
    let (corpus, manifest) = if let Ok(path) = std::env::var("CODE_RCL_BENCHMARK_CORPUS") {
        let raw = fs::read(&path)?;
        let manifest: Value = serde_json::from_slice(&fs::read(format!("{path}.manifest.json"))?)?;
        ensure!(
            format!("{:x}", Sha256::digest(&raw)) == manifest["corpus_sha256"],
            "external corpus hash mismatch"
        );
        (serde_json::from_slice(&raw)?, manifest)
    } else {
        corpus()?
    };
    let project = output.join("workspace");
    materialize(&project, &corpus, &manifest)?;
    let (threads, batch) = first_index::settings()?;
    config(&project, &model, threads, batch, &strategy, &profile)?;
    if let Some(sync) = manifest.get("sync") {
        let path = project.join(".code-rcl/config.toml");
        let mut doc: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
        set_sync_config(&mut doc, sync)?;
        fs::write(path, toml::to_string(&doc)?)?;
    }
    check_bytes(&project, &corpus)?;
    let executable = std::env::current_exe()?;
    save(
        &output.join("manifest.json"),
        &json!({"corpus":manifest,"binary_sha256":format!("{:x}",Sha256::digest(fs::read(executable)?)),"package":env!("CARGO_PKG_VERSION"),"mode":mode,"threads":threads,"batch_size":batch,"strategy":strategy,"profile":profile,"hardware":std::env::var("PROCESSOR_IDENTIFIER").ok(),"logical_cpus":std::thread::available_parallelism()?.get(),"os":std::env::consts::OS,"arch":std::env::consts::ARCH}),
    )?;
    if mode == "calibration" {
        return calibrate(&project, &output, &model);
    }
    ensure!(
        mode == "corpus" || mode == "fixture" || mode == "project",
        "invalid benchmark mode"
    );
    let fixture = if mode == "fixture" {
        for path in corpus.as_object().unwrap().keys() {
            fs::remove_file(project.join(path))?;
        }
        let mut fixture = serde_json::Map::new();
        for (language, extension) in [
            ("rust", "rs"),
            ("python", "py"),
            ("go", "go"),
            ("kotlin", "kt"),
        ] {
            for file in 0..2 {
                let mut source = if language == "go" {
                    "package fixture\n".to_string()
                } else {
                    String::new()
                };
                for n in 0..12 {
                    let statements = 3 + ((n * 7 + file * 13) % 40);
                    let name = format!("retrieve_{language}_{file}_{n}");
                    let (begin, statement, end) = match language {
                        "rust" => (
                            format!("pub fn {name}() {{\n"),
                            "    let value = 123; // indexed work\n",
                            "}\n",
                        ),
                        "python" => (
                            format!("def {name}():\n"),
                            "    value = 123 # indexed work\n",
                            "\n",
                        ),
                        "go" => (
                            format!("func {name}() {{\n"),
                            "    _ = 123 // indexed work\n",
                            "}\n",
                        ),
                        _ => (
                            format!("fun {name}() {{\n"),
                            "    println(123) // indexed work\n",
                            "}\n",
                        ),
                    };
                    source.push_str(&begin);
                    source.push_str(&statement.repeat(statements));
                    source.push_str(end);
                }
                let path = format!("fixture_{file}.{extension}");
                fs::write(project.join(&path), source.as_bytes())?;
                fixture.insert(path, json!(source));
            }
        }
        save(&output.join("fixture.json"), &json!(fixture))?;
        Some(Value::Object(fixture))
    } else {
        None
    };
    check_bytes(&project, fixture.as_ref().unwrap_or(&corpus))?;
    let mut guard = crate::progress::Guard::start(&project, Some("plain"))?;
    let started = Instant::now();
    let result = sync(&project, false)?;
    let wall = started.elapsed().as_secs_f64() * 1000.;
    let op = guard.finish("completed")?;
    save(
        &output.join("operation.json"),
        &json!({"wall_ms":wall,"result":result,"operation":op,"engine_timings_ms":crate::operation::timings()}),
    )?;
    let db = CacheDb::open(&project)?;
    let mut rows = db.embedding_chunks(None)?;
    rows.sort_by(|a, b| {
        a.chunk
            .input_hash
            .cmp(&b.chunk.input_hash)
            .then(a.chunk.symbol_key.cmp(&b.chunk.symbol_key))
            .then(a.chunk.chunk_index.cmp(&b.chunk.chunk_index))
    });
    let mut vectors = fs::File::create(output.join("vectors.f32"))?;
    let mut inputs = Vec::new();
    for row in &rows {
        for number in &row.vector {
            vectors.write_all(&number.to_le_bytes())?;
        }
        inputs.push(json!({"input_hash":row.chunk.input_hash,"text_hash":blake3::hash(row.chunk.text.as_bytes()).to_hex().to_string(),"symbol_key":row.chunk.symbol_key,"chunk_index":row.chunk.chunk_index,"representation":row.chunk.representation,"path":row.chunk.path,"source_hash":row.chunk.source_hash,"start_byte":row.chunk.start_byte,"end_byte":row.chunk.end_byte,"start_line":row.chunk.start_line,"end_line":row.chunk.end_line}));
    }
    save(&output.join("inputs.json"), &json!(inputs))?;
    check_bytes(&project, fixture.as_ref().unwrap_or(&corpus))?;
    drop(db);
    // The same frozen source and query labels are used for every strategy/profile.
    let mut queries: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/retrieval_bkndb/queries.json"
    ))?;
    if let Ok(path) = std::env::var("CODE_RCL_CONTAINER_QUERIES") {
        let extra: Value = serde_json::from_slice(&fs::read(path)?)?;
        queries["queries"]
            .as_array_mut()
            .unwrap()
            .extend(extra["queries"].as_array().unwrap().iter().cloned());
    }
    if mode == "corpus" {
        let mut rankings = Vec::new();
        for query in queries["queries"].as_array().unwrap() {
            for retrieval in ["lexical", "semantic", "hybrid"] {
                crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                    false,
                )));
                use clap::Parser;
                let mut args = vec![
                    "code-rcl",
                    "search",
                    query["query"].as_str().unwrap(),
                    "--no-sync",
                    "--limit",
                    "100",
                    "--project",
                    project.to_str().unwrap(),
                ];
                if retrieval == "semantic" {
                    args.push("--semantic");
                } else if retrieval == "hybrid" {
                    args.push("--hybrid");
                }
                let crate::cli::Command::Search(args) =
                    crate::cli::Cli::try_parse_from(args)?.command
                else {
                    unreachable!()
                };
                let start = Instant::now();
                let result = crate::commands::search::execute_search(&args)?;
                rankings.push(json!({"query":query,"mode":retrieval,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"result":result}));
            }
            save(&output.join("rankings.json"), &json!(rankings))?;
            eprintln!("evaluated {}", query["id"]);
        }
    }
    check_bytes(&project, fixture.as_ref().unwrap_or(&corpus))?;
    save(
        &output.join("validation.json"),
        &json!({"source_bytes_verified":true,"chunks":rows.len(),"dimensions":768}),
    )?;
    Ok(())
}
fn legacy_quality(project: &Path, output: &Path) -> Result<()> {
    let data: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/embedding_accuracy/dataset.json"
    ))?;
    let docs = data["documents"].as_array().unwrap();
    let queries = data["queries"].as_array().unwrap();
    with_model(project, |model| {
        let chunks = docs
            .iter()
            .enumerate()
            .map(|(i, d)| Chunk {
                path: d["path"].as_str().unwrap().into(),
                name: d["name"].as_str().unwrap().into(),
                owner: String::new(),
                kind: "function".into(),
                is_exported: false,
                symbol_key: i.to_string(),
                chunk_index: 0,
                start_byte: 0,
                end_byte: 0,
                start_line: 1,
                end_line: 1,
                source_hash: String::new(),
                input_hash: i.to_string(),
                text: format!(
                    "title: {} | text: {}",
                    d["path"].as_str().unwrap(),
                    d["text"].as_str().unwrap()
                ),
                snippet: String::new(),
                representation: None,
                symbol_range: None,
            })
            .collect::<Vec<_>>();
        let mut file = Vec::new();
        for batch in chunks.chunks(4) {
            file.extend(model.embed(batch.iter().map(|c| c.text.clone()).collect())?);
        }
        let mut length = vec![Vec::new(); chunks.len()];
        for batch in super::scheduler::benchmark_order(&chunks, model, 4)? {
            let vectors = model.embed(batch.iter().map(|i| chunks[*i].text.clone()).collect())?;
            ensure!(vectors.len() == batch.len(), "wrong legacy vector count");
            for (i, v) in batch.into_iter().zip(vectors) {
                length[i] = v;
            }
        }
        let mut qvec = Vec::new();
        for batch in queries.chunks(4) {
            qvec.extend(
                model.embed(
                    batch
                        .iter()
                        .map(|q| {
                            format!(
                                "task: code retrieval | query: {}",
                                q["text"].as_str().unwrap()
                            )
                        })
                        .collect(),
                )?,
            );
        }
        let mut results = Vec::new();
        for dim in [768, 256] {
            let file = file
                .iter()
                .map(|v| normalize(v, dim))
                .collect::<Result<Vec<_>>>()?;
            let length = length
                .iter()
                .map(|v| normalize(v, dim))
                .collect::<Result<Vec<_>>>()?;
            let minimum = file
                .iter()
                .zip(&length)
                .map(|(a, b)| cosine(a, b))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .fold(1f64, f64::min);
            let mut variants = serde_json::Map::new();
            for (name, vectors) in [("file", &file), ("length", &length)] {
                let mut ranks = Vec::new();
                for (q, vector) in queries.iter().zip(&qvec) {
                    let vector = normalize(vector, dim)?;
                    let mut scored = vectors
                        .iter()
                        .enumerate()
                        .map(|(i, v)| Ok((i, cosine(&vector, v)?)))
                        .collect::<Result<Vec<_>>>()?;
                    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                    let rank = scored
                        .iter()
                        .position(|(i, _)| *i == q["gold"].as_u64().unwrap() as usize)
                        .unwrap()
                        + 1;
                    ranks.push(rank);
                }
                variants.insert(name.into(),json!({"hit1":ranks.iter().filter(|r|**r==1).count(),"hit5":ranks.iter().filter(|r|**r<=5).count(),"ranks":ranks}));
            }
            let minimum_hits = if dim == 768 { 31 } else { 30 };
            ensure!(
                variants["file"]["hit1"].as_u64().unwrap() >= minimum_hits
                    && variants["file"]["hit5"] == 32,
                "historical model baseline regressed"
            );
            let quality = variants["length"]["hit1"].as_u64() >= variants["file"]["hit1"].as_u64()
                && variants["length"]["hit5"] == 32;
            results.push(json!({"dimensions":dim,"minimum_cosine":minimum,"cosine_passed":minimum>=0.9999,"quality_passed":quality,"variants":variants}));
        }
        save(&output.join("legacy32-quality.json"), &json!(results))?;
        save(
            &output.join("validation.json"),
            &json!({"fixture_sha256":format!("{:x}",Sha256::digest(include_bytes!("../../tests/fixtures/embedding_accuracy/dataset.json"))),"binary_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?)),"model":MODEL,"revision":REVISION}),
        )?;
        Ok(())
    })
}
fn calibrate(project: &Path, output: &Path, model_dir: &str) -> Result<()> {
    let mut db = CacheDb::open(project)?;
    crate::service::auto_sync(&mut db, project)?;
    let mut candidates = Vec::new();
    with_model(project, |model| {
        let cfg = ProjectConfig::load(project)?;
        for file in scope(&db, &cfg)? {
            if !file.parsed_ok {
                continue;
            }
            let source = db.source_snapshot(&file)?;
            let symbols = db.symbols_for_file(file.id)?;
            chunk::visit_profile(
                &file.path,
                &file.content_hash,
                &source,
                &symbols,
                model,
                &config_key(&cfg.embeddings),
                crate::config::EmbeddingChunkProfile::Full,
                &mut |c| {
                    let tokens = model.input_tokens(&c.text)?;
                    candidates.push((tokens, c));
                    Ok(())
                },
            )?;
        }
        Ok(())
    })?;
    let mut inputs = Vec::new();
    for extension in ["rs", "py", "go", "kt"] {
        let mut language: Vec<_> = candidates
            .iter()
            .filter(|(_, c)| {
                Path::new(&c.path)
                    .extension()
                    .is_some_and(|e| e == extension)
            })
            .collect();
        language.sort_by(|a, b| {
            (a.0, &a.1.path, &a.1.symbol_key, a.1.chunk_index).cmp(&(
                b.0,
                &b.1.path,
                &b.1.symbol_key,
                b.1.chunk_index,
            ))
        });
        ensure!(language.len() >= 24, "not enough {extension} chunks");
        for n in 0..24 {
            let (tokens, c) = language[n * (language.len() - 1) / 23];
            inputs.push(json!({"tokens":tokens,"chunk":c}));
        }
    }
    save(&output.join("calibration-inputs.json"), &json!(inputs))?;
    if std::env::var_os("CODE_RCL_BENCHMARK_PREPARE_ONLY").is_some() {
        let (corpus, _) = corpus()?;
        check_bytes(project, &corpus)?;
        return Ok(());
    }
    let mut records = Vec::new();
    let mut baseline: Option<Vec<Vec<f32>>> = None;
    for threads in [1, 2, 4] {
        for batch in [1, 2, 4, 8] {
            config(project, model_dir, threads, batch, "file", "full")?;
            let start = Instant::now();
            with_model(project, |model| {
                let loaded_ms = start.elapsed().as_secs_f64() * 1000.;
                let warm = Instant::now();
                model.embed(vec![inputs[0]["chunk"]["text"].as_str().unwrap().into()])?;
                let warm_ms = warm.elapsed().as_secs_f64() * 1000.;
                let token_start = Instant::now();
                let lengths = inputs
                    .iter()
                    .map(|c| model.input_tokens(c["chunk"]["text"].as_str().unwrap()))
                    .collect::<Result<Vec<_>>>()?;
                let token_ms = token_start.elapsed().as_secs_f64() * 1000.;
                let infer = Instant::now();
                let mut vectors = Vec::new();
                for group in inputs.chunks(batch) {
                    vectors.extend(
                        model.embed(
                            group
                                .iter()
                                .map(|c| c["chunk"]["text"].as_str().unwrap().into())
                                .collect(),
                        )?,
                    );
                }
                let inference_ms = infer.elapsed().as_secs_f64() * 1000.;
                ensure!(vectors.len() == 96, "calibration vector count");
                let mut min_cosine = 1f64;
                for dim in [768, 256] {
                    for (i, v) in vectors.iter().enumerate() {
                        let v = normalize(v, dim)?;
                        if let Some(old) = &baseline {
                            min_cosine = min_cosine.min(cosine(&v, &normalize(&old[i], dim)?)?);
                        }
                    }
                }
                if baseline.is_none() {
                    baseline = Some(vectors);
                }
                let padded: usize = lengths
                    .chunks(batch)
                    .map(|g| g.iter().max().unwrap() * g.len())
                    .sum();
                records.push(json!({"threads":threads,"batch_size":batch,"session_acquire_ms":loaded_ms,"warm_up_ms":warm_ms,"tokenization_ms":token_ms,"inference_ms":inference_ms,"actual_tokens":lengths.iter().sum::<usize>(),"padded_tokens":padded,"minimum_cosine_768_256":min_cosine,"cosine_gate_passed":min_cosine>=0.9999}));
                save(&output.join("calibration.json"), &json!(records))?;
                eprintln!(
                    "calibrated threads={threads} batch={batch} inference_ms={inference_ms:.0} cosine={min_cosine}"
                );
                Ok(())
            })?;
        }
    }
    let (corpus, _) = corpus()?;
    check_bytes(project, &corpus)?;
    Ok(())
}
