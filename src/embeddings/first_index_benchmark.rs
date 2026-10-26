//! Strict first-index experiment helpers. Compiled only by the ignored test harness.
use super::*;

pub(super) fn settings() -> Result<(usize, usize)> {
    let read = |name: &str| -> Result<usize> {
        Ok(std::env::var(name).unwrap_or_else(|_| "4".into()).parse()?)
    };
    let threads = read("CODE_RCL_BENCHMARK_THREADS")?;
    let batch = read("CODE_RCL_BENCHMARK_BATCH")?;
    ensure!(threads <= 64, "invalid benchmark threads (0 means auto)");
    ensure!([1, 2, 4, 8].contains(&batch), "invalid benchmark batch");
    Ok((threads, batch))
}

pub(super) fn snapshot(output: &Path) -> Result<()> {
    let source = PathBuf::from(std::env::var("CODE_RCL_BENCHMARK_SOURCE")?).canonicalize()?;
    let cfg = ProjectConfig::load(&source)?;
    let mut files = serde_json::Map::new();
    let mut hashes = serde_json::Map::new();
    for path in crate::dump::walker::collect_source_files(&source)? {
        let Some(language) = crate::analysis::Language::from_path(&path) else {
            continue;
        };
        if !language.is_supported()
            || (!cfg.sync.languages.is_empty()
                && !cfg
                    .sync
                    .languages
                    .iter()
                    .any(|l| language.matches_filter(l)))
            || fs::metadata(&path)?.len() > cfg.sync.max_file_kb.saturating_mul(1024)
        {
            continue;
        }
        let bytes = fs::read(&path)?;
        let relative = path
            .strip_prefix(&source)?
            .to_string_lossy()
            .replace('\\', "/");
        hashes.insert(
            relative.clone(),
            json!(format!("{:x}", Sha256::digest(&bytes))),
        );
        files.insert(
            relative,
            json!(String::from_utf8(bytes).context("snapshot source must be UTF-8")?),
        );
    }
    ensure!(!files.is_empty(), "snapshot has no source");
    let path = output.join("source.json");
    save(&path, &json!(files))?;
    save(
        &output.join("source.json.manifest.json"),
        &json!({
            "source_root":source,"files":hashes,"file_count":files.len(),
            "corpus_sha256":format!("{:x}",Sha256::digest(fs::read(path)?)),
            "sync":{"max_file_kb":cfg.sync.max_file_kb,"languages":cfg.sync.languages},
            "selection":"production walker ignore rules; supported languages; configured language/size scope"
        }),
    )?;
    check_bytes(&source, &json!(files))?;
    Ok(())
}

fn export(directory: &Path, inputs: &Value, vectors: &[Vec<f32>]) -> Result<()> {
    fs::create_dir_all(directory)?;
    save(&directory.join("inputs.json"), inputs)?;
    let mut stream = fs::File::create(directory.join("vectors.f32"))?;
    for vector in vectors {
        let vector = normalize(vector, 768)?;
        for v in vector {
            stream.write_all(&v.to_le_bytes())?;
        }
    }
    Ok(())
}

pub(super) fn calibration_case(output: &Path, model_dir: &str) -> Result<()> {
    let path = PathBuf::from(std::env::var("CODE_RCL_BENCHMARK_INPUTS")?);
    let raw = fs::read(path)?;
    let inputs: Value = serde_json::from_slice(&raw)?;
    let inputs = inputs.as_array().context("calibration inputs array")?;
    ensure!(inputs.len() == 96, "expected 96 frozen inputs");
    let (threads, batch) = settings()?;
    let project = output.join("workspace");
    ensure!(!project.exists(), "calibration workspace already exists");
    config(&project, model_dir, threads, batch, "file", "full")?;
    let load_start = Instant::now();
    with_model(&project, |model| {
        let acquire_ms = load_start.elapsed().as_secs_f64() * 1000.;
        let text = |v: &Value| v["chunk"]["text"].as_str().unwrap().to_owned();
        let warm = Instant::now();
        model.embed(inputs[..batch].iter().map(text).collect())?;
        let warm_ms = warm.elapsed().as_secs_f64() * 1000.;
        let token = Instant::now();
        let lengths = inputs
            .iter()
            .map(|v| model.input_tokens(&text(v)))
            .collect::<Result<Vec<_>>>()?;
        let token_ms = token.elapsed().as_secs_f64() * 1000.;
        let mut records = Vec::new();
        for repeat in 0..3 {
            let start = Instant::now();
            let mut vectors = Vec::new();
            for group in inputs.chunks(batch) {
                vectors.extend(model.embed(group.iter().map(text).collect())?);
            }
            let inference_ms = start.elapsed().as_secs_f64() * 1000.;
            ensure!(vectors.len() == inputs.len(), "calibration count mismatch");
            export(
                &output.join(format!("rep-{repeat}")),
                &json!(inputs),
                &vectors,
            )?;
            records.push(json!({"repeat":repeat,"threads":threads,"batch_size":batch,"inference_ms":inference_ms,
                "actual_tokens":lengths.iter().sum::<usize>(),"padded_tokens":lengths.chunks(batch).map(|g|g.iter().max().unwrap()*g.len()).sum::<usize>()}));
            save(
                &output.join("calibration.json"),
                &json!({"input_sha256":format!("{:x}",Sha256::digest(&raw)),"session_acquire_ms":acquire_ms,"warm_up_ms":warm_ms,"tokenization_ms":token_ms,"repeats":records}),
            )?;
            eprintln!(
                "strict calibration threads={threads} batch={batch} repeat={repeat} inference_ms={inference_ms:.0}"
            );
        }
        Ok(())
    })?;
    save(
        &output.join("validation.json"),
        &json!({"completed":true,"inputs":96,"repeats":3}),
    )?;
    Ok(())
}

pub(super) fn legacy(output: &Path, model_dir: &str, strategy: &str) -> Result<()> {
    let (threads, batch) = settings()?;
    let project = output.join("workspace");
    ensure!(!project.exists(), "legacy workspace exists");
    config(&project, model_dir, threads, batch, strategy, "full")?;
    let data: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/embedding_accuracy/dataset.json"
    ))?;
    let docs = data["documents"].as_array().unwrap();
    let queries = data["queries"].as_array().unwrap();
    with_model(&project, |model| {
        let mut chunks = Vec::new();
        for (i, d) in docs.iter().enumerate() {
            chunks.push(Chunk {
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
            });
        }
        let order = if strategy == "length" {
            super::super::scheduler::benchmark_order(&chunks, model, batch)?
        } else {
            (0..chunks.len())
                .collect::<Vec<_>>()
                .chunks(batch)
                .map(|g| g.to_vec())
                .collect()
        };
        let mut vectors = vec![Vec::new(); chunks.len()];
        for group in order {
            let values = model.embed(group.iter().map(|i| chunks[*i].text.clone()).collect())?;
            ensure!(values.len() == group.len(), "legacy count mismatch");
            for (i, v) in group.into_iter().zip(values) {
                vectors[i] = v;
            }
        }
        export(output, &json!(chunks), &vectors)?;
        let mut rankings = Vec::new();
        for (i, q) in queries.iter().enumerate() {
            let qvec = model.embed(vec![format!(
                "task: code retrieval | query: {}",
                q["text"].as_str().unwrap()
            )])?;
            for dim in [768, 256] {
                let qvec = normalize(&qvec[0], dim)?;
                let mut scored = vectors
                    .iter()
                    .enumerate()
                    .map(|(n, v)| Ok((n, cosine(&qvec, &normalize(v, dim)?)?)))
                    .collect::<Result<Vec<_>>>()?;
                scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let rank = scored
                    .iter()
                    .position(|(n, _)| *n == q["gold"].as_u64().unwrap() as usize)
                    .unwrap()
                    + 1;
                rankings.push(json!({"query":i,"dimensions":dim,"rank":rank,"metrics":{
                    "hit1":usize::from(rank==1),"hit5":usize::from(rank<=5),"mrr":1./rank as f64,
                    "ndcg5":if rank<=5 {1./((rank+1) as f64).log2()} else {0.}}}));
            }
        }
        save(&output.join("legacy-rankings.json"), &json!(rankings))?;
        Ok(())
    })?;
    save(
        &output.join("validation.json"),
        &json!({"completed":true,"queries":32,"dimensions":[768,256]}),
    )?;
    Ok(())
}
