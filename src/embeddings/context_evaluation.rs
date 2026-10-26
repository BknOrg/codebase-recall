//! Frozen-corpus Gemma-only context experiment; never rewrites the base index.
use super::*;
use crate::{cache::CacheDb, config::ProjectConfig, embeddings};
use anyhow::Context as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf, time::Instant};

fn fingerprint(db: &CacheDb) -> Result<String> {
    let mut rows = db.embedding_chunks(None)?;
    rows.sort_by(|a, b| {
        a.chunk
            .symbol_key
            .cmp(&b.chunk.symbol_key)
            .then(a.chunk.chunk_index.cmp(&b.chunk.chunk_index))
    });
    let mut hash = Sha256::new();
    for row in rows {
        hash.update(serde_json::to_vec(&(row.chunk, row.valid, row.config_key))?);
        for value in row.vector {
            hash.update(value.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[test]
#[ignore = "explicit Gemma-only purpose-context evaluation; CONTEXT_PROJECT, OUTPUT, optional VECTORS and SPLIT"]
fn gemma_context_evaluation() -> Result<()> {
    crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let project = PathBuf::from(std::env::var("CODE_RCL_CONTEXT_PROJECT")?);
    let output = PathBuf::from(std::env::var("CODE_RCL_CONTEXT_OUTPUT")?);
    let split = std::env::var("CODE_RCL_CONTEXT_SPLIT").unwrap_or_else(|_| "tuning".into());
    let budget = std::env::var("CODE_RCL_CONTEXT_TOKENS")
        .unwrap_or_else(|_| "128".into())
        .parse::<usize>()?;
    ensure!(
        matches!(split.as_str(), "tuning" | "evaluation"),
        "invalid split"
    );
    ensure!(!output.exists(), "choose a fresh query output");
    fs::create_dir_all(&output)?;
    let corpus: Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/retrieval_bkndb/corpus.json"
    ))?;
    let db = CacheDb::open(&project)?;
    let cfg = ProjectConfig::load(&project)?;
    ensure!(
        cfg.embeddings.dimensions == 768,
        "probe uses full 768-dimensional stored vectors"
    );
    let key = embeddings::config_key(&cfg.embeddings);
    let before = fingerprint(&db)?;
    let original = db
        .embedding_chunks(None)?
        .into_iter()
        .filter(|r| r.valid && r.config_key == key)
        .collect::<Vec<_>>();
    let mut files = db.all_files()?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for file in &files {
        ensure!(
            db.source_snapshot(file)?
                == corpus[&file.path]
                    .as_str()
                    .context("source absent from corpus")?,
            "snapshot mismatch: {}",
            file.path
        );
    }
    let vectors_dir = std::env::var("CODE_RCL_CONTEXT_VECTORS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| output.join("purpose-vectors"));
    fs::create_dir_all(&vectors_dir)?;
    let build = Instant::now();
    let mut contexts = vec![];
    let mut symbols = BTreeMap::<String, Value>::new();
    for row in &original {
        let c = &row.chunk;
        symbols.entry(c.symbol_key.clone()).or_insert_with(
            || json!({"symbol_key":c.symbol_key,"name":c.name,"kind":c.kind,"path":c.path}),
        );
    }
    embeddings::with_model(&project, |model| {
        for file in files.iter().filter(|f| f.parsed_ok) {
            let source = db.source_snapshot(file)?;
            let declarations = db.symbols_for_file(file.id)?;
            let tree = crate::analysis::embedding_bodies::parse_tree(&file.path, &source);
            let bodies = tree.as_ref().map(crate::analysis::embedding_bodies::bodies);
            let facts = tree
                .as_ref()
                .map(|t| super::syntax(&source, &declarations, t))
                .unwrap_or_default();
            for symbol in &declarations {
                if let Some(context) = super::build(
                    &file.path,
                    &file.content_hash,
                    &source,
                    symbol,
                    &declarations,
                    bodies.as_ref(),
                    facts.get(&symbol.id),
                    model,
                    &key,
                    budget,
                )? {
                    symbols.entry(context.symbol_key.clone()).or_insert_with(||json!({"symbol_key":context.symbol_key,"name":context.name,"kind":context.kind,"path":context.path}));
                    contexts.push(context);
                }
            }
        }
        let mut lengths = vec![];
        for context in contexts.drain(..) {
            lengths.push((model.input_tokens(&context.text)?, context));
        }
        lengths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.symbol_key.cmp(&b.1.symbol_key)));
        contexts = lengths.into_iter().map(|(_, c)| c).collect();
        let inputs = serde_json::to_vec(&contexts)?;
        let path = vectors_dir.join("inputs.json");
        if path.exists() {
            ensure!(fs::read(&path)? == inputs, "cached purpose inputs changed");
        } else {
            fs::write(&path, &inputs)?;
        }
        let path = vectors_dir.join("vectors.f32");
        let bytes = if path.exists() {
            fs::read(&path)?
        } else {
            vec![]
        };
        ensure!(bytes.len() % (768 * 4) == 0, "incomplete cached vector");
        let mut vectors = bytes
            .as_chunks::<{ 768 * 4 }>()
            .0
            .iter()
            .map(|r| {
                r.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        ensure!(vectors.len() <= contexts.len(), "too many cached vectors");
        let mut stream = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        for batch in contexts[vectors.len()..].chunks(cfg.embeddings.batch_size.max(1)) {
            crate::operation::check()?;
            let embedded = model.embed(batch.iter().map(|c| c.text.clone()).collect())?;
            ensure!(
                embedded.len() == batch.len(),
                "purpose vector count mismatch"
            );
            for row in embedded {
                let vector = embeddings::normalize(&row, 768)?;
                for value in &vector {
                    stream.write_all(&value.to_le_bytes())?;
                }
                vectors.push(vector);
            }
            stream.flush()?;
            eprintln!("Gemma purpose vectors {}/{}", vectors.len(), contexts.len());
        }
        let build_ms = build.elapsed().as_secs_f64() * 1000.0;
        let symbols = symbols.values().cloned().collect::<Vec<_>>();
        fs::write(output.join("symbols.json"), serde_json::to_vec(&symbols)?)?;
        let indices = symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (s["symbol_key"].as_str().unwrap(), i))
            .collect::<BTreeMap<_, _>>();
        let mut queries = vec![];
        for raw in [
            include_str!("../../tests/fixtures/retrieval_bkndb/queries.json"),
            include_str!("../../tests/fixtures/retrieval_bkndb/container-queries.json"),
            include_str!("../../tests/fixtures/retrieval_bkndb/negative-queries.json"),
        ] {
            let doc: Value = serde_json::from_str(raw)?;
            queries.extend(
                doc["queries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|q| q["split"] == split)
                    .cloned(),
            );
        }
        let mut scores_file = fs::File::create(output.join("scores.jsonl"))?;
        for query in &queries {
            let started = Instant::now();
            let text = query["query"].as_str().unwrap();
            let q = model.embed(vec![
                format!("task: code retrieval | query: {text}"),
                format!("task: search result | query: {text}"),
            ])?;
            ensure!(q.len() == 2, "query vector count mismatch");
            let q = q
                .iter()
                .map(|v| embeddings::normalize(v, 768))
                .collect::<Result<Vec<_>>>()?;
            let mut evidence = vec![[None::<f64>; 4]; symbols.len()];
            for row in &original {
                let i = indices[row.chunk.symbol_key.as_str()];
                for j in 0..2 {
                    let v = embeddings::cosine(&q[j], &row.vector)?;
                    evidence[i][j] = Some(evidence[i][j].map_or(v, |old| old.max(v)));
                }
            }
            for (context, vector) in contexts.iter().zip(&vectors) {
                let i = indices[context.symbol_key.as_str()];
                for j in 0..2 {
                    evidence[i][j + 2] = Some(embeddings::cosine(&q[j], vector)?);
                }
            }
            writeln!(
                scores_file,
                "{}",
                serde_json::to_string(
                    &json!({"query":query,"evidence":evidence,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
                )?
            )?;
            scores_file.flush()?;
            eprintln!("Gemma context {split} {}", query["id"]);
        }
        ensure!(
            fingerprint(&db)? == before,
            "base input/vector bytes changed"
        );
        fs::write(
            output.join("completed.json"),
            serde_json::to_vec_pretty(
                &json!({"split":split,"queries":queries.len(),"context_tokens":budget,"purpose_contexts":contexts.len(),"base_chunks":original.len(),"base_vectors_unchanged":true,"source_bytes_verified":true,"context_build_ms":build_ms,"model":embeddings::MODEL,"revision":embeddings::REVISION,"context_version":VERSION,"vectors_directory":vectors_dir,"context_vectors_sha256":format!("{:x}",Sha256::digest(fs::read(path)?)),"context_inputs_sha256":format!("{:x}",Sha256::digest(inputs)),"test_executable_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?))}),
            )?,
        )?;
        Ok(())
    })?;
    ensure!(
        fingerprint(&db)? == before,
        "base input/vector bytes changed"
    );
    Ok(())
}

#[test]
#[ignore = "explicit Gemma-only 80-document historical comparison: CONTEXT_PROJECT, CONTEXT_OUTPUT"]
fn gemma_historical_evaluation() -> Result<()> {
    crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let project = PathBuf::from(std::env::var("CODE_RCL_CONTEXT_PROJECT")?);
    let output = PathBuf::from(std::env::var("CODE_RCL_CONTEXT_OUTPUT")?);
    ensure!(!output.exists(), "choose a fresh output");
    fs::create_dir_all(&output)?;
    let raw = include_bytes!("../../tests/fixtures/embedding_accuracy/dataset.json");
    let dataset: Value = serde_json::from_slice(raw)?;
    let options = crate::commands::search::retrieval::profile();
    embeddings::with_model(&project, |model| {
        let mut body_inputs = vec![];
        let mut purpose_inputs = vec![];
        for d in dataset["documents"].as_array().unwrap() {
            let path = d["path"].as_str().unwrap();
            let name = d["name"].as_str().unwrap();
            let text = d["text"].as_str().unwrap();
            ensure!(
                text.lines().next() == Some(path),
                "historical path header mismatch"
            );
            let (_, source) = text.split_once('\n').context("historical source missing")?;
            let parsed = crate::analysis::parse_file(crate::analysis::Language::Rust, source);
            let mut symbols = parsed
                .symbols
                .into_iter()
                .enumerate()
                .map(|(i, s)| crate::cache::models::SymbolRow {
                    id: i as i64,
                    file_id: 0,
                    name: s.name,
                    kind: s.kind,
                    parent_symbol_id: s.parent_index.map(|i| i as i64),
                    is_exported: s.is_exported,
                    start_line: Some(s.start_line),
                    end_line: Some(s.end_line),
                    start_byte: Some(s.start_byte),
                    end_byte: Some(s.end_byte),
                    signature: s.signature,
                    param_count: s.param_count,
                    type_name: s.type_name,
                })
                .collect::<Vec<_>>();
            // This historical fixture intentionally contains truncated source
            // fragments. Keep those bytes and declared identities unchanged;
            // never invent closing code to make their AST parse successfully.
            if !symbols.iter().any(|s| s.name == name) {
                ensure!(
                    d["snippet_truncated"] == true,
                    "untruncated historical declaration absent"
                );
                symbols.push(crate::cache::models::SymbolRow {
                    id: -1,
                    file_id: 0,
                    name: name.into(),
                    kind: "function".into(),
                    parent_symbol_id: None,
                    is_exported: false,
                    start_line: Some(1),
                    end_line: Some(source.lines().count() as i64),
                    start_byte: Some(0),
                    end_byte: Some(source.len() as i64),
                    signature: source.lines().next().map(str::to_owned),
                    param_count: None,
                    type_name: None,
                });
            }
            let symbol = symbols
                .iter()
                .find(|s| s.name == name)
                .with_context(|| format!("historical symbol absent: {name}"))?;
            let tree = crate::analysis::embedding_bodies::parse_tree(path, source);
            let bodies = tree.as_ref().map(crate::analysis::embedding_bodies::bodies);
            let facts = tree
                .as_ref()
                .map(|tree| syntax(source, &symbols, tree))
                .unwrap_or_default();
            let c = build(
                path,
                "historical",
                source,
                symbol,
                &symbols,
                bodies.as_ref(),
                facts.get(&symbol.id),
                model,
                "historical",
                options.context_tokens,
            )?
            .context("historical context absent")?;
            body_inputs.push(format!("title: {path} | text: {text}"));
            purpose_inputs.push(c.text);
        }
        let mut embed = |inputs: &[String]| -> Result<Vec<Vec<f32>>> {
            let mut vectors = vec![];
            for batch in inputs.chunks(4) {
                for v in model.embed(batch.to_vec())? {
                    vectors.push(embeddings::normalize(&v, 768)?);
                }
            }
            Ok(vectors)
        };
        let body = embed(&body_inputs)?;
        let purpose = embed(&purpose_inputs)?;
        ensure!(
            body.len() == 80 && purpose.len() == 80,
            "historical vector count mismatch"
        );
        let mut rows = vec![];
        for query in dataset["queries"].as_array().unwrap() {
            let text = query["text"].as_str().unwrap();
            let q = model.embed(vec![
                format!("task: code retrieval | query: {text}"),
                format!("task: search result | query: {text}"),
            ])?;
            let q = q
                .iter()
                .map(|v| embeddings::normalize(v, 768))
                .collect::<Result<Vec<_>>>()?;
            let mut before = vec![];
            let mut after = vec![];
            for i in 0..80 {
                let evidence = [
                    Some(embeddings::cosine(&q[0], &body[i])?),
                    Some(embeddings::cosine(&q[1], &body[i])?),
                    Some(embeddings::cosine(&q[0], &purpose[i])?),
                    Some(embeddings::cosine(&q[1], &purpose[i])?),
                ];
                before.push((i, evidence[0].unwrap()));
                after.push((i, options.ranking.score("function", evidence).unwrap()));
            }
            for ranked in [&mut before, &mut after] {
                ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            }
            let gold = query["gold"].as_u64().unwrap() as usize;
            rows.push(json!({"query":query,"before_rank":before.iter().position(|r|r.0==gold).unwrap()+1,"after_rank":after.iter().position(|r|r.0==gold).unwrap()+1,"before":before,"after":after}));
        }
        let count = |key: &str, k: u64| {
            rows.iter()
                .filter(|r| r[key].as_u64().unwrap() <= k)
                .count()
        };
        fs::write(
            output.join("historical.json"),
            serde_json::to_vec_pretty(
                &json!({"documents":80,"queries":32,"profile":options,"before":{"hit1":count("before_rank",1),"hit5":count("before_rank",5)},"after":{"hit1":count("after_rank",1),"hit5":count("after_rank",5)},"rows":rows,"fixture_sha256":format!("{:x}",Sha256::digest(raw)),"executable_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?)),"model":embeddings::MODEL,"revision":embeddings::REVISION,"context_inputs":purpose_inputs}),
            )?,
        )?;
        Ok(())
    })
}
