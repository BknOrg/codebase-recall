//! Full Gemma-only pipeline validation on immutable source bytes.
use super::*;
use anyhow::bail;
use clap::Parser;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, time::Instant};

fn begin() {
    crate::operation::begin(Arc::new(std::sync::atomic::AtomicBool::new(false)));
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fingerprint(db: &CacheDb) -> Result<String> {
    let mut rows = db.embedding_chunks(None)?;
    rows.sort_by(|a, b| {
        a.chunk
            .symbol_key
            .cmp(&b.chunk.symbol_key)
            .then(a.chunk.chunk_index.cmp(&b.chunk.chunk_index))
    });
    let mut h = Sha256::new();
    for r in rows {
        h.update(serde_json::to_vec(&(r.chunk, r.config_key, r.valid))?);
        for v in r.vector {
            h.update(v.to_le_bytes());
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
fn source_check(corpus: &BTreeMap<String, String>, db: &CacheDb, project: &Path) -> Result<()> {
    for file in db.all_files()? {
        let source = corpus.get(&file.path).context("unexpected source file")?;
        ensure!(
            fs::read(project.join(&file.path))? == source.as_bytes(),
            "disk source mismatch: {}",
            file.path
        );
        ensure!(
            db.source_snapshot(&file)? == *source,
            "snapshot mismatch: {}",
            file.path
        );
    }
    Ok(())
}
fn span(
    corpus: &BTreeMap<String, String>,
    path: &str,
    start: usize,
    end: usize,
    line: usize,
    end_line: usize,
    text: &str,
) -> Result<()> {
    let source = corpus.get(path).context("missing source span")?;
    ensure!(
        source.get(start..end) == Some(text),
        "excerpt differs from source: {path}:{start}"
    );
    ensure!(
        line == source[..start].bytes().filter(|b| *b == b'\n').count() + 1,
        "incorrect start line"
    );
    ensure!(
        end_line == source[..end].bytes().filter(|b| *b == b'\n').count() + 1,
        "incorrect end line"
    );
    Ok(())
}

#[test]
#[ignore = "explicit Gemma pipeline benchmark: CODE_RCL_SEARCH_OUTPUT, SEED, CONTEXT_VECTORS, optional PROFILE/SPLIT"]
fn search_pipeline_evaluation() -> Result<()> {
    begin();
    let output = PathBuf::from(std::env::var("CODE_RCL_SEARCH_OUTPUT")?);
    ensure!(!output.exists(), "choose a fresh output directory");
    let seed = PathBuf::from(std::env::var("CODE_RCL_SEARCH_SEED")?);
    let inputs = PathBuf::from(std::env::var("CODE_RCL_SEARCH_CONTEXT_VECTORS")?);
    let split = std::env::var("CODE_RCL_SEARCH_SPLIT").unwrap_or_else(|_| "evaluation".into());
    let limit = std::env::var("CODE_RCL_SEARCH_LIMIT")
        .unwrap_or_else(|_| "25".into())
        .parse::<usize>()?;
    ensure!((1..=100).contains(&limit), "invalid benchmark output limit");
    ensure!(
        matches!(split.as_str(), "tuning" | "evaluation" | "fresh"),
        "invalid split"
    );
    let options: Options = if let Ok(p) = std::env::var("CODE_RCL_SEARCH_PROFILE") {
        serde_json::from_slice(&fs::read(p)?)?
    } else {
        profile()
    };
    let raw = include_bytes!("../../tests/fixtures/retrieval_bkndb/corpus.json");
    let corpus: BTreeMap<String, String> = serde_json::from_slice(raw)?;
    let project = output.join("workspace");
    fs::create_dir_all(project.join(".code-rcl"))?;
    for (path, source) in &corpus {
        let p = project.join(path);
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(p, source.as_bytes())?;
    }
    for file in ["cache.bkndb", "config.toml"] {
        fs::copy(
            seed.join(".code-rcl").join(file),
            project.join(".code-rcl").join(file),
        )?;
    }
    let mut db = CacheDb::open(&project)?;
    let before = fingerprint(&db)?;
    crate::service::auto_sync(&mut db, &project)?;
    ensure!(
        fingerprint(&db)? == before,
        "source sync changed legacy embeddings"
    );
    source_check(&corpus, &db, &project)?;
    let cfg = crate::config::ProjectConfig::load(&project)?;
    let key = embeddings::config_key(&cfg.embeddings);
    let contexts: Vec<embeddings::context::Context> =
        serde_json::from_slice(&fs::read(inputs.join("inputs.json"))?)?;
    let bytes = fs::read(inputs.join("vectors.f32"))?;
    ensure!(
        bytes.len() == contexts.len() * 768 * 4,
        "context vector count mismatch"
    );
    let mut by_file: HashMap<String, Vec<_>> = HashMap::new();
    for (c, b) in contexts
        .into_iter()
        .zip(bytes.as_chunks::<{ 768 * 4 }>().0.iter())
    {
        let v = b
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>();
        by_file.entry(c.path.clone()).or_default().push((c, v));
    }
    for file in db.all_files()?.into_iter().filter(|f| f.parsed_ok) {
        let marker = embeddings::context::marker(&db, &file, &key, options.context_tokens)?;
        db.commit_purpose_file(
            &file,
            &key,
            &marker,
            &by_file.remove(&file.path).unwrap_or_default(),
        )?;
    }
    let mut queries = vec![];
    for raw in [
        include_str!("../../tests/fixtures/retrieval_bkndb/queries.json"),
        include_str!("../../tests/fixtures/retrieval_bkndb/container-queries.json"),
        include_str!("../../tests/fixtures/retrieval_bkndb/negative-queries.json"),
        include_str!("../../tests/fixtures/retrieval_bkndb/fresh-feature-queries.json"),
    ] {
        let v: Value = serde_json::from_str(raw)?;
        queries.extend(
            v["queries"]
                .as_array()
                .context("missing queries")?
                .iter()
                .filter(|q| q["split"] == split)
                .cloned(),
        );
    }
    let start = Instant::now();
    if let Ok(maximum) = std::env::var("CODE_RCL_SEARCH_MAX_QUERIES") {
        queries.truncate(maximum.parse()?);
    }
    embeddings::with_model(&project, |model| {
        model.embed(vec!["task: code retrieval | query: warmup".into()])?;
        let cold_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut stream = fs::File::create(output.join("rankings.jsonl"))?;
        for q in &queries {
            for mode in ["semantic", "hybrid"] {
                begin();
                let parsed = crate::cli::Cli::try_parse_from([
                    "code-rcl",
                    "search",
                    q["query"].as_str().unwrap(),
                    if mode == "semantic" {
                        "--semantic"
                    } else {
                        "--hybrid"
                    },
                    "--project",
                    project.to_str().unwrap(),
                ])?;
                let crate::cli::Command::Search(mut args) = parsed.command else {
                    bail!("invalid command")
                };
                args.limit = limit;
                let started = Instant::now();
                crate::service::auto_sync(&mut db, &project)?;
                let mut trace = Trace::default();
                let result =
                    execute_traced(&mut db, &project, &args, model, &options, Some(&mut trace))?;
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
                for h in result.matches.as_ref().unwrap() {
                    span(
                        &corpus,
                        &h.path,
                        h.start_byte,
                        h.end_byte,
                        h.line,
                        h.end_line,
                        &h.snippet,
                    )?;
                    for s in &h.supporting_locations {
                        span(
                            &corpus,
                            &s.path,
                            s.start_byte,
                            s.end_byte,
                            s.line,
                            s.end_line,
                            &s.snippet,
                        )?;
                    }
                }
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(
                        &json!({"query":q,"mode":mode,"profile":options,"ranking":result.matches,"trace":trace,"elapsed_ms":elapsed_ms,"metadata":{"retrieval":crate::operation::retrieval_metadata()}})
                    )?
                )?;
                stream.flush()?;
                eprintln!("Gemma pipeline {mode} {}", q["id"]);
            }
        }
        source_check(&corpus, &db, &project)?;
        ensure!(
            fingerprint(&db)? == before,
            "pipeline rewrote base embeddings"
        );
        fs::write(
            output.join("completed.json"),
            serde_json::to_vec_pretty(
                &json!({"split":split,"source_bytes_verified":true,"base_vectors_unchanged":true,"rank_limit":limit,"model":embeddings::MODEL,"revision":embeddings::REVISION,"profile":options,"corpus_sha256":sha(raw),"context_inputs_sha256":sha(&fs::read(inputs.join("inputs.json"))?),"context_vectors_sha256":sha(&bytes),"test_executable_sha256":sha(&fs::read(std::env::current_exe()?)?),"cold_load_and_warmup_ms":cold_ms,"latency_includes_source_sync":true,"rankings_sha256":sha(&fs::read(output.join("rankings.jsonl"))?)}),
            )?,
        )?;
        Ok(())
    })
}

#[test]
#[ignore = "explicit JS/TS/Java Gemma validation and edit refresh: SEARCH_OUTPUT, SEARCH_SEED"]
fn supplemental_pipeline_evaluation() -> Result<()> {
    begin();
    let output = PathBuf::from(std::env::var("CODE_RCL_SEARCH_OUTPUT")?);
    ensure!(!output.exists(), "choose a fresh output");
    let project = output.join("workspace");
    fs::create_dir_all(project.join(".code-rcl"))?;
    let seed = PathBuf::from(std::env::var("CODE_RCL_SEARCH_SEED")?);
    fs::copy(
        seed.join(".code-rcl/config.toml"),
        project.join(".code-rcl/config.toml"),
    )?;
    let corpus = BTreeMap::from([
        (
            "js/runner.js".into(),
            include_str!("../../tests/fixtures/search_features/js/runner.js").into(),
        ),
        (
            "js/journal.js".into(),
            include_str!("../../tests/fixtures/search_features/js/journal.js").into(),
        ),
        (
            "ts/coordinator.ts".into(),
            include_str!("../../tests/fixtures/search_features/ts/coordinator.ts").into(),
        ),
        (
            "ts/storage.ts".into(),
            include_str!("../../tests/fixtures/search_features/ts/storage.ts").into(),
        ),
        (
            "java/TransactionJob.java".into(),
            include_str!("../../tests/fixtures/search_features/java/TransactionJob.java").into(),
        ),
        (
            "java/WriteJournal.java".into(),
            include_str!("../../tests/fixtures/search_features/java/WriteJournal.java").into(),
        ),
    ]);
    source_materialize(&project, &corpus)?;
    let mut db = CacheDb::open(&project)?;
    crate::service::auto_sync(&mut db, &project)?;
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/search_features/queries.json"
    ))?;
    let options = profile();
    let mut rows = vec![];
    embeddings::with_model(&project, |model| {
        embeddings::index_with(&mut db, &project, false, model)?;
        let selection = Selection::new(&project, &Default::default())?;
        embeddings::context::refresh(&db, &project, &selection, model, options.context_tokens)?;
        for query in fixture["queries"].as_array().unwrap() {
            for mode in ["semantic", "hybrid"] {
                begin();
                let parsed = crate::cli::Cli::try_parse_from([
                    "code-rcl",
                    "search",
                    query["query"].as_str().unwrap(),
                    if mode == "semantic" {
                        "--semantic"
                    } else {
                        "--hybrid"
                    },
                ])?;
                let crate::cli::Command::Search(args) = parsed.command else {
                    bail!("invalid command")
                };
                let start = Instant::now();
                crate::service::auto_sync(&mut db, &project)?;
                let result = execute_with(&mut db, &project, &args, model, &options)?;
                let elapsed = start.elapsed().as_secs_f64() * 1000.;
                for h in result.matches.as_ref().unwrap() {
                    span(
                        &corpus,
                        &h.path,
                        h.start_byte,
                        h.end_byte,
                        h.line,
                        h.end_line,
                        &h.snippet,
                    )?;
                    for s in &h.supporting_locations {
                        span(
                            &corpus,
                            &s.path,
                            s.start_byte,
                            s.end_byte,
                            s.line,
                            s.end_line,
                            &s.snippet,
                        )?;
                    }
                }
                rows.push(json!({"query":query,"mode":mode,"ranking":result.matches,"elapsed_ms":elapsed,"metadata":{"retrieval":crate::operation::retrieval_metadata()}}));
            }
        }
        source_check(&corpus, &db, &project)?;
        let mut edited = corpus.clone();
        edited
            .get_mut("js/runner.js")
            .unwrap()
            .push_str("\n// refreshed source snapshot\n");
        fs::write(
            project.join("js/runner.js"),
            edited["js/runner.js"].as_bytes(),
        )?;
        begin();
        let start = Instant::now();
        crate::service::auto_sync(&mut db, &project)?;
        let parsed = crate::cli::Cli::try_parse_from([
            "code-rcl",
            "search",
            "execute a JavaScript job",
            "--semantic",
        ])?;
        let crate::cli::Command::Search(args) = parsed.command else {
            bail!("invalid command")
        };
        execute_with(&mut db, &project, &args, model, &options)?;
        let edited_ms = start.elapsed().as_secs_f64() * 1000.;
        let refresh = crate::operation::retrieval_metadata();
        begin();
        execute_with(&mut db, &project, &args, model, &options)?;
        let unchanged = crate::operation::retrieval_metadata();
        ensure!(
            unchanged["index_refresh"].is_null() && unchanged["context_refresh"].is_null(),
            "unchanged search repeated index work"
        );
        source_check(&edited, &db, &project)?;
        fs::write(
            output.join("supplemental.json"),
            serde_json::to_vec_pretty(
                &json!({"profile":options,"rows":rows,"distractors":fixture["distractors"],"edited_refresh_ms":edited_ms,"edited_refresh":refresh,"unchanged_search":unchanged,"source_bytes_verified":true,"model":embeddings::MODEL,"revision":embeddings::REVISION,"executable_sha256":sha(&fs::read(std::env::current_exe()?)?)}),
            )?,
        )?;
        Ok(())
    })
}

fn source_materialize(project: &Path, corpus: &BTreeMap<String, String>) -> Result<()> {
    for (path, source) in corpus {
        let p = project.join(path);
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(p, source.as_bytes())?;
    }
    Ok(())
}
