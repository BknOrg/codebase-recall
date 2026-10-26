use super::*;
use clap::Parser;
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
struct Mock {
    calls: usize,
    hook: Option<Box<dyn FnMut()>>,
    invalid: bool,
}
impl Embedder for Mock {
    fn offsets(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        Ok(text
            .char_indices()
            .map(|(i, c)| (i, i + c.len_utf8()))
            .collect())
    }
    fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.calls += texts.len();
        if let Some(h) = self.hook.as_mut() {
            h();
        }
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0; 768];
                v[if t.contains("alpha") {
                    0
                } else if t.contains("beta") {
                    1
                } else {
                    2
                }] = if self.invalid { f32::NAN } else { 1.0 };
                v
            })
            .collect())
    }
}
fn begin() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    crate::operation::begin(flag.clone());
    flag
}

#[test]
fn purpose_context_refresh_is_atomic_reuses_inputs_and_tracks_source_changes() {
    let (dir, mut db) = fixture();
    let selection = crate::selection::Selection::new(dir.path(), &Default::default()).unwrap();
    let mut model = Mock::default();
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    let base = db
        .embedding_chunks(None)
        .unwrap()
        .into_iter()
        .map(|r| (r.chunk.input_hash, r.vector))
        .collect::<Vec<_>>();
    let initial = context::refresh(&db, dir.path(), &selection, &mut model, 128).unwrap();
    assert_eq!(initial["embedded_contexts"], 2);
    assert_eq!(
        context::valid_vectors(&db, dir.path(), 128).unwrap().len(),
        2
    );
    let calls = model.calls;
    assert_eq!(
        context::refresh(&db, dir.path(), &selection, &mut model, 128).unwrap()["updated_files"],
        0
    );
    assert_eq!(model.calls, calls);
    assert_eq!(
        db.embedding_chunks(None)
            .unwrap()
            .into_iter()
            .map(|r| (r.chunk.input_hash, r.vector))
            .collect::<Vec<_>>(),
        base
    );
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn alpha() {}\nfn beta() {}\n\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(
        context::valid_vectors(&db, dir.path(), 128)
            .unwrap()
            .is_empty()
    );
    crate::cache::purpose::FAIL_COMMIT.with(|f| f.set(true));
    assert!(context::refresh(&db, dir.path(), &selection, &mut model, 128).is_err());
    assert_eq!(db.purpose_rows().unwrap().len(), 2);
    assert_eq!(
        context::pending(&db, dir.path(), &selection, 128)
            .unwrap()
            .len(),
        1
    );
    let result = context::refresh(&db, dir.path(), &selection, &mut model, 128).unwrap();
    assert_eq!(result["embedded_contexts"], 0);
    assert_eq!(result["reused_contexts"], 2);
    assert_eq!(model.calls, calls);
    drop(db);
    let mut db = CacheDb::open(dir.path()).unwrap();
    assert_eq!(
        context::valid_vectors(&db, dir.path(), 128).unwrap().len(),
        2
    );
    fs::rename(dir.path().join("lib.rs"), dir.path().join("renamed.rs")).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(db.purpose_rows().unwrap().is_empty());
    context::refresh(&db, dir.path(), &selection, &mut model, 128).unwrap();
    fs::remove_file(dir.path().join("renamed.rs")).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(db.purpose_rows().unwrap().is_empty());
}

#[test]
fn purpose_context_cancellation_and_invalid_vectors_never_publish() {
    let (dir, db) = fixture();
    let selection = crate::selection::Selection::new(dir.path(), &Default::default()).unwrap();
    let flag = begin();
    let cancel = flag.clone();
    let mut model = Mock {
        hook: Some(Box::new(move || cancel.store(true, Ordering::Relaxed))),
        ..Default::default()
    };
    assert!(context::refresh(&db, dir.path(), &selection, &mut model, 128).is_err());
    begin();
    assert!(db.purpose_rows().unwrap().is_empty());
    let mut model = Mock {
        invalid: true,
        ..Default::default()
    };
    assert!(context::refresh(&db, dir.path(), &selection, &mut model, 128).is_err());
    assert!(db.purpose_rows().unwrap().is_empty());
    assert_eq!(
        context::pending(&db, dir.path(), &selection, 128)
            .unwrap()
            .len(),
        1
    );
}
fn fixture() -> (tempfile::TempDir, CacheDb) {
    begin();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn alpha() {}\nfn beta() {}\n",
    )
    .unwrap();
    let mut db = CacheDb::open(dir.path()).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    (dir, db)
}
fn args(project: &Path, extra: &[&str]) -> crate::cli::SearchArgs {
    let mut cli = vec![
        "code-rcl",
        "search",
        "alpha",
        "--semantic",
        "--project",
        project.to_str().unwrap(),
    ];
    cli.extend_from_slice(extra);
    match crate::cli::Cli::try_parse_from(cli).unwrap().command {
        crate::cli::Command::Search(a) => a,
        _ => unreachable!(),
    }
}
#[test]
fn documentation_backfill_reuses_exact_embedding_inputs_and_vector_bytes() {
    let (dir, mut db) = fixture();
    let source = "/// commit only after the closure succeeds\n\npub fn alpha() {}\n";
    fs::write(dir.path().join("lib.rs"), source).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let mut model = Mock::default();
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    let fingerprint = |db: &CacheDb| {
        db.embedding_chunks(None)
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.chunk.input_hash,
                    r.chunk.text,
                    r.vector.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
                    r.valid,
                    r.config_key,
                )
            })
            .collect::<Vec<_>>()
    };
    let before = fingerprint(&db);
    assert!(
        !before[0].1.contains("closure succeeds"),
        "preserve the existing embedding input format"
    );
    let calls = model.calls;
    let file = &db.all_files().unwrap()[0];
    db.db
        .write_tx(|tx| {
            tx.relational()
                .table(db.tables.search_files.clone())
                .upsert(
                    [
                        ("file_id".into(), file.id.into()),
                        (
                            "marker".into(),
                            format!("content-v2:{}", file.content_hash).into(),
                        ),
                        ("documents".into(), 1i64.into()),
                        ("length".into(), 1i64.into()),
                    ]
                    .into_iter()
                    .collect(),
                )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        crate::service::auto_sync(&mut db, dir.path())
            .unwrap()
            .changed,
        0
    );
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    assert_eq!(model.calls, calls);
    assert_eq!(fingerprint(&db), before);
    assert_eq!(
        db.content_candidates("closure succeeds", |d| d.kind == "function", 10)
            .unwrap()[0]
            .0
            .name,
        "alpha"
    );
}
#[test]
fn incremental_invalidation_reuse_deletion_and_restart() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(m.calls, 2);
    let generation = status(&db, dir.path(), true).unwrap()["index_generation"].clone();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(m.calls, 2);
    assert_eq!(
        status(&db, dir.path(), true).unwrap()["index_generation"],
        generation
    );
    // Adding a declaration reallocates symbol IDs but retains unchanged chunk inputs.
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(db.embedding_chunks(None).unwrap().iter().all(|r| !r.valid));
    assert!(semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m).is_err());
    let result = index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(result["reused_chunks"], 2);
    assert_eq!(result["embedded_chunks"], 1);
    drop(db);
    let mut db = CacheDb::open(dir.path()).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
    fs::remove_file(dir.path().join("lib.rs")).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(db.embedding_chunks(None).unwrap().is_empty());
}
#[test]
fn file_and_symbol_renames_never_return_old_embeddings_after_restart() {
    let (dir, mut db) = fixture();
    let mut model = Mock::default();
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    fs::rename(dir.path().join("lib.rs"), dir.path().join("renamed.rs")).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert!(db.embedding_chunks(None).unwrap().is_empty());
    assert!(semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut model).is_err());

    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    drop(db);
    let mut db = CacheDb::open(dir.path()).unwrap();
    let hits = semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut model).unwrap();
    assert!(hits.iter().all(|hit| hit.path == "renamed.rs"));
    assert!(hits.iter().any(|hit| hit.name == "alpha"));

    fs::write(
        dir.path().join("renamed.rs"),
        "pub fn omega() {}\nfn beta() {}\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    drop(db);
    let mut db = CacheDb::open(dir.path()).unwrap();
    assert!(semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut model).is_err());
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    let hits = semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut model).unwrap();
    assert!(
        hits.iter()
            .all(|hit| hit.name != "alpha" && hit.path == "renamed.rs")
    );
    assert!(hits.iter().any(|hit| hit.name == "omega"));
}

#[test]
fn same_length_edit_invalidates_only_changed_file() {
    let (dir, mut db) = fixture();
    fs::write(dir.path().join("other.rs"), "pub fn other() {}\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let path = dir.path().join("lib.rs");
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "pub fn omega() {}\nfn beta() {}\n").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before))
        .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let state = status(&db, dir.path(), true).unwrap();
    assert_eq!(state["pending_files"], 1);
    assert_eq!(state["indexed_chunks"], 1);
    let hits = semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m).unwrap();
    assert!(hits.iter().all(|h| h.path == "other.rs"));
    assert!(crate::operation::metadata().1);
    assert!(!crate::operation::graph_partial());
}
#[test]
fn source_race_does_not_commit_or_mark_complete() {
    let (dir, mut db) = fixture();
    let path = dir.path().join("lib.rs");
    let mut m = Mock {
        hook: Some(Box::new(move || {
            fs::write(&path, "fn raced() {}\n").unwrap();
        })),
        ..Default::default()
    };
    assert!(
        index_with(&mut db, dir.path(), false, &mut m)
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    assert!(db.embedding_chunks(None).unwrap().is_empty());
    assert_eq!(
        db.meta_get("embedding_complete").unwrap().as_deref(),
        Some("false")
    );
}
#[test]
fn configuration_race_and_invalid_vectors_do_not_commit() {
    let (dir, mut db) = fixture();
    let config = dir.path().join(".code-rcl/config.toml");
    let mut m = Mock {
        hook: Some(Box::new(move || {
            fs::write(&config, "[embeddings]\ndimensions=256\n").unwrap()
        })),
        ..Default::default()
    };
    assert!(index_with(&mut db, dir.path(), false, &mut m).is_err());
    assert!(db.embedding_chunks(None).unwrap().is_empty());
    fs::remove_file(dir.path().join(".code-rcl/config.toml")).unwrap();
    let mut m = Mock {
        invalid: true,
        ..Default::default()
    };
    assert!(index_with(&mut db, dir.path(), false, &mut m).is_err());
    assert!(db.embedding_chunks(None).unwrap().is_empty());
}
#[test]
fn cancellation_preserves_prior_commits_and_resumes() {
    let (dir, mut db) = fixture();
    fs::write(dir.path().join("second.rs"), "fn second() {}\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let cancel = begin();
    let mut m = Mock {
        hook: Some(Box::new(move || cancel.store(true, Ordering::Relaxed))),
        ..Default::default()
    };
    assert!(index_with(&mut db, dir.path(), false, &mut m).is_err());
    assert_eq!(
        db.meta_get("embedding_complete").unwrap().as_deref(),
        Some("false")
    );
    begin();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
}
#[test]
fn dimension_change_requires_explicit_rebuild() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    fs::write(
        dir.path().join(".code-rcl/config.toml"),
        "[embeddings]\ndimensions=256\n",
    )
    .unwrap();
    assert!(index_with(&mut db, dir.path(), false, &mut m).is_err());
    assert!(semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m).is_err());
    index_with(&mut db, dir.path(), true, &mut m).unwrap();
    assert!(
        db.embedding_chunks(None)
            .unwrap()
            .iter()
            .all(|r| r.vector.len() == 256)
    );
}

#[test]
fn schema_10_migration_and_partial_snapshot_backfill_preserve_vectors_and_symbol_ids() {
    let (dir, mut db) = fixture();
    fs::write(dir.path().join("other.rs"), "pub fn other() {}\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let mut model = Mock::default();
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    let rows_before = db.embedding_chunks(None).unwrap();
    let ids_before = db
        .all_files()
        .unwrap()
        .iter()
        .flat_map(|f| db.symbols_for_file(f.id).unwrap())
        .map(|s| s.id)
        .collect::<Vec<_>>();
    let generation = db.meta_get("generation").unwrap();
    let embedding_generation = db.meta_get("embedding_generation").unwrap();
    db.db
        .write_tx(|b| {
            let mut rel = b.relational();
            for table in [
                "source_snapshots",
                "search_documents",
                "search_postings",
                "search_files",
                "purpose_contexts",
                "purpose_files",
            ] {
                rel.drop_table(table)?;
            }
            Ok(())
        })
        .unwrap();
    db.meta_set("schema_version", "10").unwrap();
    drop(db);
    let db = CacheDb::open(dir.path()).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap().as_deref(),
        Some("12")
    );
    let files = db.all_files().unwrap();
    assert!(files.iter().all(|f| db.source_snapshot(f).is_err()));
    // Simulate an interrupted backfill after one independently committed file.
    db.ensure_source_snapshot(
        &files[0],
        &fs::read(dir.path().join(&files[0].path)).unwrap(),
    )
    .unwrap();
    drop(db);
    let mut db = CacheDb::open(dir.path()).unwrap();
    let stats = crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert_eq!((stats.changed, stats.unchanged), (0, 2));
    let calls = model.calls;
    index_with(&mut db, dir.path(), false, &mut model).unwrap();
    assert_eq!(calls, model.calls);
    assert_eq!(generation, db.meta_get("generation").unwrap());
    assert_eq!(
        embedding_generation,
        db.meta_get("embedding_generation").unwrap()
    );
    let ids_after = db
        .all_files()
        .unwrap()
        .iter()
        .flat_map(|f| db.symbols_for_file(f.id).unwrap())
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(ids_before, ids_after);
    let fingerprints = |rows: Vec<crate::cache::embeddings::StoredChunk>| {
        let mut values = rows
            .into_iter()
            .map(|r| {
                (
                    r.chunk.input_hash,
                    r.valid,
                    r.config_key,
                    r.vector.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        values.sort();
        values
    };
    assert_eq!(
        fingerprints(rows_before),
        fingerprints(db.embedding_chunks(None).unwrap())
    );
}

#[test]
fn cached_and_direct_source_produce_identical_model_inputs_across_languages_and_profiles() {
    let (dir, mut db) = fixture();
    for (path, text) in [
        (
            "lib.rs",
            format!(
                "// outside 日本語\r\n/// doc\r\npub fn alpha() {{ let s = \"{}\"; }}\r\n",
                "🙂".repeat(1200)
            ),
        ),
        (
            "tool.py",
            format!(
                "# outside 日本語\r\ndef alpha():\r\n    return \"{}\"\r\n",
                "🙂".repeat(1200)
            ),
        ),
        (
            "tool.go",
            format!(
                "package app\n// doc 日本語\nfunc alpha() string {{ return \"{}\" }}\n",
                "🙂".repeat(1200)
            ),
        ),
        (
            "tool.kt",
            format!(
                "// outside 日本語\nfun alpha(): String {{ return \"{}\" }}\n",
                "🙂".repeat(1200)
            ),
        ),
    ] {
        fs::write(dir.path().join(path), text).unwrap();
    }
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let model = Mock::default();
    let cfg = ProjectConfig::load(dir.path()).unwrap();
    for file in db.all_files().unwrap() {
        assert!(file.parsed_ok, "{}", file.path);
        let symbols = db.symbols_for_file(file.id).unwrap();
        let direct = fs::read_to_string(dir.path().join(&file.path)).unwrap();
        let cached = db.source_snapshot(&file).unwrap();
        assert_eq!(direct.as_bytes(), cached.as_bytes());
        for profile in [
            crate::config::EmbeddingChunkProfile::Full,
            crate::config::EmbeddingChunkProfile::Compact,
        ] {
            let key = config_key(&cfg.embeddings);
            let old = chunk::profile_chunks(
                &file.path,
                &file.content_hash,
                &direct,
                &symbols,
                &model,
                &key,
                profile,
            )
            .unwrap();
            let new = chunk::profile_chunks(
                &file.path,
                &file.content_hash,
                &cached,
                &symbols,
                &model,
                &key,
                profile,
            )
            .unwrap();
            assert!(!old.is_empty());
            assert_eq!(
                serde_json::to_value(&old).unwrap(),
                serde_json::to_value(&new).unwrap()
            );
            for (a, b) in old.iter().zip(new.iter()) {
                assert_eq!(
                    model.input_tokens(&a.text).unwrap(),
                    model.input_tokens(&b.text).unwrap()
                );
            }
        }
    }
}

#[test]
fn missing_source_snapshot_fails_without_falling_back_to_repo_text_in_both_batchers() {
    for batching in ["file", "length"] {
        let (dir, mut db) = fixture();
        fs::write(
            dir.path().join(".code-rcl/config.toml"),
            format!("[embeddings]\nbatching='{batching}'\n"),
        )
        .unwrap();
        let file_id = db.all_files().unwrap()[0].id;
        db.db
            .write_tx(|b| {
                b.relational()
                    .table(db.tables.source_snapshots.clone())
                    .delete(&file_id.into())?;
                Ok(())
            })
            .unwrap();
        let mut model = Mock::default();
        assert!(
            index_with(&mut db, dir.path(), false, &mut model)
                .unwrap_err()
                .to_string()
                .contains("source snapshot missing")
        );
        assert_eq!(model.calls, 0);
        begin();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        index_with(&mut db, dir.path(), false, &mut model).unwrap();
        assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
    }
}
#[test]
fn unicode_chunks_obey_token_budget_overlap_and_source_ranges() {
    let (dir, mut db) = fixture();
    let source = format!(
        "/// documentation\npub fn alpha() {{ let text = \"{}\"; }}",
        "日本語🙂".repeat(500)
    );
    fs::write(dir.path().join("lib.rs"), &source).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let file = db.all_files().unwrap().remove(0);
    let m = Mock::default();
    let chunks = chunk::chunks(
        &file.path,
        &file.content_hash,
        &source,
        &db.symbols_for_file(file.id).unwrap(),
        &m,
        &config_key(&EmbeddingsConfig::default()),
    )
    .unwrap();
    assert!(chunks.len() > 2);
    for c in &chunks {
        assert!(m.offsets(&c.text).unwrap().len() <= 510);
        assert_eq!(&source[c.start_byte..c.end_byte], c.snippet);
        assert!(source.is_char_boundary(c.start_byte) && source.is_char_boundary(c.end_byte));
    }
    for pair in chunks.windows(2) {
        assert!(pair[1].start_byte < pair[0].end_byte);
        assert!(pair[1].start_byte > pair[0].start_byte);
    }
}
#[test]
fn filtering_deduplication_normalization_and_empty_results() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let mut a = args(dir.path(), &[]);
    a.exported = true;
    let hits = semantic_with(&db, dir.path(), &a, &mut m).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "alpha");
    assert_eq!(hits[0].cosine_score, Some(1.0));
    a.kind = Some("struct".into());
    assert!(
        semantic_with(&db, dir.path(), &a, &mut m)
            .unwrap()
            .is_empty()
    );
    for d in [256, 768] {
        let v = normalize(&vec![1.0; 768], d).unwrap();
        assert_eq!(v.len(), d);
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);
    }
    assert!(normalize(&vec![0.0; 768], 768).is_err());
    assert!(normalize(&vec![f32::INFINITY; 768], 768).is_err());
}
#[test]
fn rrf_deduplicates_branches_and_has_stable_ties() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let hits = semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m).unwrap();
    let fused = fuse(hits.clone(), vec![hits[0].clone()]);
    assert_eq!(fused.len(), hits.len());
    assert_eq!(fused[0].rrf_score, Some(2.0 / 61.0));
    assert_eq!(fused[0].retrieval, "hybrid");
    let tied = fuse(vec![hits[1].clone()], vec![hits[0].clone()]);
    assert_eq!(tied[0].name, "alpha");
    assert_eq!(tied[0].rrf_score, tied[1].rrf_score);
}
#[test]
fn metadata_only_changes_and_empty_symbol_files_do_not_reembed() {
    let (dir, mut db) = fixture();
    fs::write(dir.path().join("empty.rs"), "// no declarations\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let calls = m.calls;
    fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("lib.rs"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
        .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(m.calls, calls);
    assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
}

#[test]
fn switching_manual_and_auto_threads_preserves_chunk_identity_and_vector_bytes() {
    for batching in ["file", "length"] {
        let (dir, mut db) = fixture();
        let config = dir.path().join(".code-rcl/config.toml");
        fs::write(
            &config,
            format!("[embeddings]\nthreads=4\nbatching='{batching}'\n"),
        )
        .unwrap();
        let mut model = Mock::default();
        index_with(&mut db, dir.path(), false, &mut model).unwrap();
        let calls = model.calls;
        let snapshot = |db: &CacheDb| {
            db.embedding_chunks(None)
                .unwrap()
                .into_iter()
                .map(|r| {
                    assert!(r.valid);
                    (
                        (
                            r.chunk.path,
                            r.chunk.symbol_key,
                            r.chunk.chunk_index,
                            r.chunk.input_hash,
                        ),
                        r.vector.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let before = snapshot(&db);
        assert!(!before.is_empty());
        for threads in [0, 4] {
            fs::write(
                &config,
                format!("[embeddings]\nthreads={threads}\nbatching='{batching}'\n"),
            )
            .unwrap();
            crate::service::auto_sync(&mut db, dir.path()).unwrap();
            index_with(&mut db, dir.path(), false, &mut model).unwrap();
            assert_eq!(model.calls, calls);
            assert_eq!(before, snapshot(&db));
            assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
        }
    }
}
#[test]
fn out_of_scope_files_are_not_searchable() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    fs::write(
        dir.path().join(".code-rcl/config.toml"),
        "[sync]\nlanguages=[\"python\"]\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["indexed_chunks"], 0);
    assert!(semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m).is_err());
}

#[test]
fn a_cancelled_later_file_preserves_an_earlier_commit() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let previous = db.embedding_chunks(None).unwrap().len();
    fs::write(dir.path().join("later.rs"), "fn later() {}\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let cancel = begin();
    let mut m = Mock {
        hook: Some(Box::new(move || cancel.store(true, Ordering::Relaxed))),
        ..Default::default()
    };
    assert!(index_with(&mut db, dir.path(), false, &mut m).is_err());
    assert_eq!(db.embedding_chunks(None).unwrap().len(), previous);
    begin();
    index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 0);
}
#[cfg(feature = "embeddings")]
#[test]
#[ignore = "requires explicitly downloaded pinned model; set CODE_RCL_EMBEDDING_MODEL_DIR"]
fn embedding_model_accuracy() {
    begin();
    let model_dir = std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR")
        .expect("set CODE_RCL_EMBEDDING_MODEL_DIR to pinned model directory");
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".code-rcl")).unwrap();
    fs::write(
        d.path().join(".code-rcl/config.toml"),
        format!(
            "[embeddings]\nmodel_dir={}\n",
            serde_json::to_string(&model_dir).unwrap()
        ),
    )
    .unwrap();
    let data: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/embedding_accuracy/dataset.json"
    ))
    .unwrap();
    let docs = data["documents"].as_array().unwrap();
    let queries = data["queries"].as_array().unwrap();
    with_model(d.path(), |model| {
        let inputs: Vec<_> = docs
            .iter()
            .map(|v| {
                format!(
                    "title: {} | text: {}",
                    v["path"].as_str().unwrap(),
                    v["text"].as_str().unwrap()
                )
            })
            .collect();
        let start = std::time::Instant::now();
        let mut vectors = vec![];
        for batch in inputs.chunks(4) {
            vectors.extend(model.embed(batch.to_vec())?);
        }
        eprintln!("80 documents: {:?}", start.elapsed());
        let inputs: Vec<_> = queries
            .iter()
            .map(|v| {
                format!(
                    "task: code retrieval | query: {}",
                    v["text"].as_str().unwrap()
                )
            })
            .collect();
        let start = std::time::Instant::now();
        let mut qvec = vec![];
        for batch in inputs.chunks(4) {
            qvec.extend(model.embed(batch.to_vec())?);
        }
        eprintln!("32 queries: {:?}", start.elapsed());
        for (dim, minimum) in [(768, 31), (256, 30)] {
            let dv: Vec<_> = vectors
                .iter()
                .map(|v| normalize(v, dim))
                .collect::<Result<_>>()?;
            let mut at1 = 0;
            let mut at5 = 0;
            for (i, q) in queries.iter().enumerate() {
                let v = normalize(&qvec[i], dim)?;
                let mut scored = dv
                    .iter()
                    .enumerate()
                    .map(|(j, d)| Ok((j, cosine(&v, d)?)))
                    .collect::<Result<Vec<_>>>()?;
                scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let rank = scored
                    .iter()
                    .position(|(j, _)| *j == q["gold"].as_u64().unwrap() as usize)
                    .unwrap()
                    + 1;
                eprintln!("{dim}d query={i} rank={rank}");
                at1 += usize::from(rank == 1);
                at5 += usize::from(rank <= 5);
            }
            assert!(
                at1 >= minimum,
                "{dim}d regression: {at1}/32 first, baseline {minimum}"
            );
            assert_eq!(at5, 32);
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn interrupted_terminal_state_is_partial_even_when_files_committed() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    db.meta_set("embedding_complete", "false").unwrap();
    let state = status(&db, dir.path(), true).unwrap();
    assert_eq!(state["pending_files"], 0);
    assert_eq!(state["complete"], false);
    assert_eq!(state["freshness"], "partial");
    assert!(
        !semantic_with(&db, dir.path(), &args(dir.path(), &[]), &mut m)
            .unwrap()
            .is_empty()
    );
    assert!(crate::operation::metadata().1);
    begin();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["complete"], true);
}

#[test]
fn oversized_previously_indexed_files_are_excluded_and_can_reenter() {
    let (dir, mut db) = fixture();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    fs::write(
        dir.path().join(".code-rcl/config.toml"),
        "[sync]\nmax_file_kb=1\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        format!("pub fn alpha() {{ /* {} */ }}", "x".repeat(2048)),
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let state = status(&db, dir.path(), true).unwrap();
    assert_eq!(state["excluded_files"], 1);
    assert_eq!(state["indexed_chunks"], 0);
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn alpha() {}\nfn beta() {}\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["pending_files"], 1);
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    assert_eq!(status(&db, dir.path(), true).unwrap()["excluded_files"], 0);
}
#[test]
fn indexing_respects_the_effective_language_scope_of_source_sync() {
    let (dir, mut db) = fixture();
    fs::write(dir.path().join("other.py"), "def beta():\n    return 1\n").unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let mut m = Mock::default();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    fs::write(dir.path().join("other.py"), "def beta():\n    return 999\n").unwrap();
    let parsed = crate::cli::Cli::try_parse_from([
        "code-rcl",
        "sync",
        "--project",
        dir.path().to_str().unwrap(),
        "--language",
        "rust",
    ])
    .unwrap();
    let crate::cli::Command::Sync(sync) = parsed.command else {
        unreachable!()
    };
    crate::commands::sync::sync_cache(&mut db, &sync).unwrap();
    index_with(&mut db, dir.path(), false, &mut m).unwrap();
    let state = status(&db, dir.path(), true).unwrap();
    assert_eq!(state["excluded_files"], 1);
    assert!(
        semantic_with(&db, dir.path(), &args(dir.path(), &["--no-sync"]), &mut m)
            .unwrap()
            .iter()
            .all(|h| h.path != "other.py")
    );
}

#[test]
fn semantic_filters_are_applied_before_candidate_limits() {
    let (dir, mut db) = fixture();
    fs::create_dir(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("tests/alpha.py"),
        "def alpha():\n    return 1\n",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    let hits = semantic_with(
        &db,
        dir.path(),
        &args(
            dir.path(),
            &["--language", "python", "--path", "tests/**", "--limit", "1"],
        ),
        &mut Mock::default(),
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.path == "tests/alpha.py"));
    let hits = semantic_with(
        &db,
        dir.path(),
        &args(dir.path(), &["--exclude-path", "tests/**"]),
        &mut Mock::default(),
    )
    .unwrap();
    assert!(hits.iter().all(|h| h.path == "lib.rs"));
}

#[test]
#[ignore = "developer profiling; no model needed"]
fn profile_status_row_decoding() {
    let (d, mut db) = fixture();
    fs::write(
        d.path().join("many.rs"),
        (0..2000)
            .map(|i| format!("pub fn symbol_{i}() {{}}\n"))
            .collect::<String>(),
    )
    .unwrap();
    crate::service::auto_sync(&mut db, d.path()).unwrap();
    index_with(&mut db, d.path(), false, &mut Mock::default()).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(status(&db, d.path(), false).unwrap());
    }
    println!("status 2002 chunks, 20 calls: {:?}", start.elapsed());
}

#[test]
fn blocked_mock_inference_heartbeats_cancels_and_resumes() {
    use std::sync::mpsc;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn alpha() {}\n").unwrap();
    let path = dir.path().to_path_buf();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (release, paused) = mpsc::channel();
    let (send, events) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::operation::begin(worker_cancel);
        crate::progress::sink(Some(Arc::new(move |v| {
            let _ = send.send(v);
        })));
        let mut guard = crate::progress::Guard::start(&path, None).unwrap();
        let mut db = CacheDb::open(&path).unwrap();
        crate::service::auto_sync(&mut db, &path).unwrap();
        let mut mock = Mock {
            hook: Some(Box::new(move || {
                paused.recv_timeout(Duration::from_secs(10)).unwrap();
            })),
            ..Default::default()
        };
        assert!(index_with(&mut db, &path, false, &mut mock).is_err());
        assert_ne!(
            db.meta_get("embedding_complete").unwrap().as_deref(),
            Some("true")
        );
        guard.finish("cancelled").unwrap();
        crate::progress::sink(None);
    });
    loop {
        let event = events.recv_timeout(Duration::from_secs(10)).unwrap();
        if event["phase"] == "inference" {
            assert_eq!(event["committed_chunks"], 0);
            break;
        }
    }
    assert_eq!(
        crate::progress::read(dir.path()).unwrap().unwrap()["active"],
        true
    );
    assert!(CacheDb::open(dir.path()).is_err());
    cancel.store(true, Ordering::Relaxed);
    release.send(()).unwrap();
    worker.join().unwrap();
    begin();
    let mut db = CacheDb::open(dir.path()).unwrap();
    assert!(
        index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap()["index"]["complete"]
            .as_bool()
            .unwrap()
    );
    assert!(
        !semantic_with(
            &db,
            dir.path(),
            &args(dir.path(), &[]),
            &mut Mock::default()
        )
        .unwrap()
        .is_empty()
    );
}
#[test]
fn legacy_counter_upgrade_and_invalidations_match_row_scan() {
    let (d, mut db) = fixture();
    index_with(&mut db, d.path(), false, &mut Mock::default()).unwrap();
    let fast = status(&db, d.path(), false).unwrap();
    db.meta_set("embedding_count_format", "0").unwrap();
    let legacy = status(&db, d.path(), false).unwrap();
    assert_eq!(legacy, fast);
    db.ensure_embedding_counts().unwrap();
    fs::write(d.path().join("lib.rs"), "pub fn gamma() {}\n").unwrap();
    crate::service::auto_sync(&mut db, d.path()).unwrap();
    let state = status(&db, d.path(), true).unwrap();
    assert_eq!(state["indexed_chunks"], 0);
    assert!(state["invalid_chunks"].as_u64().unwrap() > 0);
    db.meta_set("embedding_count_format", "0").unwrap();
    assert_eq!(status(&db, d.path(), true).unwrap(), state);
}

#[test]
fn no_op_prepared_index_still_requires_an_available_model() {
    let (d, mut db) = fixture();
    index_with(&mut db, d.path(), false, &mut Mock::default()).unwrap();
    fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[embeddings]\nmodel_dir='missing-model'\n",
    )
    .unwrap();
    assert!(index_prepared(&mut db, d.path(), false).is_err());
}

fn length_mode(dir: &Path, extra: &str) {
    fs::write(
        dir.join(".code-rcl/config.toml"),
        format!("[embeddings]\nbatching='length'\n{extra}"),
    )
    .unwrap();
}
#[test]
fn length_batches_preserve_inputs_vectors_order_and_allow_rollback_without_rebuild() {
    struct Fingerprint;
    impl Embedder for Fingerprint {
        fn offsets(&self, text: &str) -> Result<Vec<(usize, usize)>> {
            Mock::default().offsets(text)
        }
        fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    let hash = blake3::hash(text.as_bytes());
                    (0..768)
                        .map(|i| f32::from(hash.as_bytes()[i % 32]) + 1.)
                        .collect()
                })
                .collect())
        }
    }
    let (dir, mut db) = fixture();
    for n in 0..12 {
        fs::write(
            dir.path().join(format!("f{n:02}.rs")),
            format!(
                "pub fn f{n}() {{ let x = \"{}\"; }}",
                "日本語".repeat(n * 10 + 1)
            ),
        )
        .unwrap();
    }
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    index_with(&mut db, dir.path(), false, &mut Fingerprint).unwrap();
    let baseline = db.embedding_chunks(None).unwrap();
    let old_key = db.meta_get("embedding_config").unwrap();
    length_mode(dir.path(), "");
    let result = index_with(&mut db, dir.path(), true, &mut Fingerprint).unwrap();
    assert_eq!(old_key, db.meta_get("embedding_config").unwrap());
    assert_eq!(result["batching"], "length");
    assert!(result["scheduler_peak_bytes"].as_u64().unwrap() <= 64 * 1024 * 1024);
    let mut expected = baseline
        .iter()
        .map(|r| {
            (
                &r.chunk.input_hash,
                &r.chunk.text,
                &r.chunk.snippet,
                &r.vector,
            )
        })
        .collect::<Vec<_>>();
    let rows = db.embedding_chunks(None).unwrap();
    let mut actual = rows
        .iter()
        .map(|r| {
            (
                &r.chunk.input_hash,
                &r.chunk.text,
                &r.chunk.snippet,
                &r.vector,
            )
        })
        .collect::<Vec<_>>();
    expected.sort_by_key(|r| r.0);
    actual.sort_by_key(|r| r.0);
    assert_eq!(expected, actual);
    let mut mock = Mock::default();
    assert_eq!(
        index_with(&mut db, dir.path(), false, &mut mock).unwrap()["embedded_chunks"],
        0
    );
    assert_eq!(mock.calls, 0);
    fs::remove_file(dir.path().join(".code-rcl/config.toml")).unwrap();
    assert_eq!(
        index_with(&mut db, dir.path(), false, &mut mock).unwrap()["embedded_chunks"],
        0
    );
}
#[test]
fn length_deduplicates_repeated_windows_and_reuses_vectors_after_source_edit() {
    let (dir, mut db) = fixture();
    fs::write(
        dir.path().join("lib.rs"),
        format!("fn alpha() {{ let s = \"{}\"; }}", "x".repeat(120_000)),
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    length_mode(dir.path(), "");
    let result = index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    assert!(result["deduplicated_chunks"].as_u64().unwrap() > 0);
    assert_eq!(
        result["embedded_chunks"].as_u64().unwrap(),
        result["unique_inferences"].as_u64().unwrap()
            + result["deduplicated_chunks"].as_u64().unwrap()
    );
    let source = fs::read_to_string(dir.path().join("lib.rs")).unwrap() + "\nfn beta() {}";
    fs::write(dir.path().join("lib.rs"), source).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let result = index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    assert!(result["reused_chunks"].as_u64().unwrap() > 100);
    assert_eq!(result["embedded_chunks"], 1);
}
#[test]
fn length_cancel_race_failure_cleanup_and_resume() {
    for failure in ["source", "config", "cancel", "vector", "staging"] {
        let (dir, mut db) = fixture();
        fs::write(dir.path().join("other.rs"), "fn other() {}\n").unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        length_mode(dir.path(), "");
        let flag = begin();
        let path = dir.path().to_path_buf();
        let mut mock = Mock {
            invalid: failure == "vector",
            hook: Some(Box::new(move || match failure {
                "source" => fs::write(path.join("lib.rs"), "fn changed() {}").unwrap(),
                "config" => fs::write(
                    path.join(".code-rcl/config.toml"),
                    "[embeddings]\nbatching='length'\ndimensions=256",
                )
                .unwrap(),
                "cancel" => flag.store(true, Ordering::Relaxed),
                "staging" => {
                    let root = path.join(".code-rcl/embedding-staging");
                    for operation in fs::read_dir(root).unwrap() {
                        for child in fs::read_dir(operation.unwrap().path()).unwrap() {
                            let child = child.unwrap();
                            if child.file_type().unwrap().is_dir() {
                                fs::remove_dir_all(child.path()).unwrap();
                            }
                        }
                    }
                }
                _ => {}
            })),
            ..Default::default()
        };
        assert!(
            index_with(&mut db, dir.path(), false, &mut mock).is_err(),
            "{failure}"
        );
        assert!(db.embedding_chunks(None).unwrap().is_empty(), "{failure}");
        assert_eq!(
            fs::read_dir(dir.path().join(".code-rcl/embedding-staging"))
                .unwrap()
                .count(),
            0
        );
        begin();
        length_mode(dir.path(), "");
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert_eq!(
            index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap()["index"]["complete"],
            true
        );
    }
}
#[test]
fn staged_commit_read_failure_is_atomic_and_recovery_preserves_foreign_directories() {
    let (dir, mut db) = fixture();
    index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    let before = db.embedding_chunks(None).unwrap().len();
    let row = db.embedding_chunks(None).unwrap().remove(0);
    let file = db.all_files().unwrap().remove(0);
    let key = config_key(&EmbeddingsConfig::default());
    let rows = vec![
        Ok((row.chunk, row.vector)),
        Err(anyhow::anyhow!("staging read failure")),
    ];
    assert!(
        db.commit_embedding_stream(&file.path, &file.content_hash, &key, 2, rows)
            .is_err()
    );
    assert_eq!(db.embedding_chunks(None).unwrap().len(), before);
    let root = dir.path().join(".code-rcl/embedding-staging");
    fs::create_dir_all(root.join("operation-foreign")).unwrap();
    fs::write(root.join("operation-foreign/keep"), "foreign").unwrap();
    let interrupted = staging::Staging::new(dir.path()).unwrap();
    let owned = interrupted.abandon();
    assert!(owned.exists());
    let recovery = staging::Staging::new(dir.path()).unwrap();
    assert!(!owned.exists());
    assert!(root.join("operation-foreign/keep").exists());
    drop(recovery);
}

#[test]
fn compact_ast_preserves_members_defaults_unicode_and_source_provenance() {
    let fixtures = [
        (
            "a.rs",
            "struct Boxed { value: i32 } impl Boxed { fn new() -> Self { let text = \"日本語 } {\"; Self { value: 7 } } fn run(&self) { /* } */ let body_marker = 99; } }",
        ),
        (
            "a.py",
            "class Outer:\n    value = 7\n    class Inner:\n        def run(self):\n            body_marker = '} { 日本語'\n            return body_marker\n    def run(self):\n        return self.value\n",
        ),
        (
            "a.kt",
            "class Outer {\n    val value: () -> Int = { 7 }\n    class Inner {\n        fun run(): String {\n            val body_marker = \"日本語 } {\"\n            return body_marker\n        }\n    }\n    fun run(): Int { return value() }\n}",
        ),
        (
            "a.java",
            "interface Api { void run(); } class Outer { int value = 7; class Inner { String run() { String body_marker = \"日本語 } {\"; return body_marker; } } int run() { return value; } }",
        ),
        (
            "a.ts",
            "class Outer { value = () => 7; run(): string { const body_marker = '日本語 } {'; return body_marker; } }",
        ),
    ];
    for (path, source) in fixtures {
        begin();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(path), source).unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        let file = db.all_files().unwrap().remove(0);
        assert!(file.parsed_ok, "{path}");
        let symbols = db.symbols_for_file(file.id).unwrap();
        let rows = chunk::profile_chunks(
            path,
            &file.content_hash,
            source,
            &symbols,
            &Mock::default(),
            "compact-test",
            crate::config::EmbeddingChunkProfile::Compact,
        )
        .unwrap();
        let summaries = rows
            .iter()
            .filter(|c| c.representation.as_deref() == Some("container_summary"))
            .collect::<Vec<_>>();
        assert!(!summaries.is_empty(), "{path}");
        for c in &rows {
            assert_eq!(c.snippet, source[c.start_byte..c.end_byte], "{path}");
            assert!(Mock::default().input_tokens(&c.text).unwrap() <= 512);
        }
        assert!(summaries.iter().any(|c| c.text.contains("value")), "{path}");
        if path != "a.rs" {
            assert!(
                summaries.iter().any(|c| c.text.contains("7")),
                "field initializer lost: {path}"
            );
        }
        assert!(
            summaries.iter().all(|c| !c.text.contains("body_marker")),
            "{path}"
        );
        assert!(
            rows.iter()
                .filter(|c| c.representation.is_none())
                .any(|c| c.text.contains("body_marker")),
            "{path}"
        );
        assert!(summaries.iter().all(|c| c.symbol_range.is_some()));
    }
}
#[test]
fn compact_ambiguous_ast_falls_back_and_profile_change_requires_full_rebuild() {
    let (dir, mut db) = fixture();
    fs::write(
        dir.path().join("lib.rs"),
        "struct A {} impl A { fn run() { let body_marker = 42; } }",
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let file = db.all_files().unwrap().remove(0);
    let symbols = db.symbols_for_file(file.id).unwrap();
    let source = fs::read_to_string(dir.path().join("lib.rs")).unwrap();
    let full = chunk::chunks(
        "a.vue",
        &file.content_hash,
        &source,
        &symbols,
        &Mock::default(),
        "same-key",
    )
    .unwrap();
    let compact = chunk::profile_chunks(
        "a.vue",
        &file.content_hash,
        &source,
        &symbols,
        &Mock::default(),
        "same-key",
        crate::config::EmbeddingChunkProfile::Compact,
    )
    .unwrap();
    assert_eq!(
        full.iter().map(|c| &c.text).collect::<Vec<_>>(),
        compact.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    assert!(
        crate::operation::container_stats()["fallbacks"]
            .as_u64()
            .unwrap()
            > 0
    );
    index_with(&mut db, dir.path(), false, &mut Mock::default()).unwrap();
    length_mode(dir.path(), "chunk_profile='compact'\n");
    assert!(
        index_with(&mut db, dir.path(), false, &mut Mock::default())
            .unwrap_err()
            .to_string()
            .contains("--full")
    );
    index_with(&mut db, dir.path(), true, &mut Mock::default()).unwrap();
    assert!(
        db.embedding_chunks(None)
            .unwrap()
            .iter()
            .all(|c| c.config_key.contains("symbol-v2-compact"))
    );
    assert!(
        semantic_with(
            &db,
            dir.path(),
            &args(dir.path(), &[]),
            &mut Mock::default()
        )
        .unwrap()
        .iter()
        .any(|m| m.representation.is_some())
    );
}
#[test]
fn length_wrong_model_count_never_publishes_an_incomplete_file() {
    struct Wrong(Mock);
    impl Embedder for Wrong {
        fn offsets(&self, s: &str) -> Result<Vec<(usize, usize)>> {
            self.0.offsets(s)
        }
        fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            let mut v = self.0.embed(texts)?;
            v.pop();
            Ok(v)
        }
    }
    let (dir, mut db) = fixture();
    length_mode(dir.path(), "");
    let error = index_with(&mut db, dir.path(), false, &mut Wrong(Mock::default())).unwrap_err();
    assert!(error.to_string().contains("wrong item count"));
    assert!(db.embedding_chunks(None).unwrap().is_empty());
    assert_eq!(status(&db, dir.path(), true).unwrap()["complete"], false);
}
#[test]
fn staged_file_results_can_exceed_scheduler_buffer() {
    let (dir, mut db) = fixture();
    fs::write(
        dir.path().join("lib.rs"),
        format!("fn alpha() {{ let s = \"{}\"; }}", "abcxyz".repeat(35_000)),
    )
    .unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    length_mode(dir.path(), "");
    let cfg = ProjectConfig::load(dir.path()).unwrap();
    let key = config_key(&cfg.embeddings);
    let budget = 512 * 1024;
    let result = scheduler::index_bounded(
        &db,
        dir.path(),
        false,
        &mut Mock::default(),
        &cfg,
        &key,
        budget,
    )
    .unwrap();
    assert!(result["scheduler_peak_bytes"].as_u64().unwrap() <= budget as u64);
    let rows = db.embedding_chunks(None).unwrap();
    assert!(rows.len() * 768 * 4 > budget);
    assert_eq!(
        result["embedded_chunks"].as_u64().unwrap() as usize,
        rows.len()
    );
}
#[test]
fn interrupted_staging_is_recovered_before_model_loading_even_after_rollback_to_file_mode() {
    let (dir, _db) = fixture();
    let stage = staging::Staging::new(dir.path()).unwrap();
    let old = stage.abandon();
    assert!(old.exists());
    let mut guard = crate::progress::Guard::start(dir.path(), Some("off")).unwrap();
    assert!(!old.exists());
    guard.finish("completed").unwrap();
}
#[test]
fn compact_token_counters_include_retained_adjacent_documentation() {
    let (dir, mut db) = fixture();
    let source = "/// Retain this documentation. 日本語\nstruct Config { value: i32 }\n";
    fs::write(dir.path().join("lib.rs"), source).unwrap();
    crate::service::auto_sync(&mut db, dir.path()).unwrap();
    let file = db.all_files().unwrap().remove(0);
    begin();
    let chunks = chunk::profile_chunks(
        &file.path,
        &file.content_hash,
        source,
        &db.symbols_for_file(file.id).unwrap(),
        &Mock::default(),
        "compact",
        crate::config::EmbeddingChunkProfile::Compact,
    )
    .unwrap();
    assert!(
        chunks
            .iter()
            .any(|c| c.text.contains("Retain this documentation"))
    );
    let counters = crate::operation::container_stats();
    assert_eq!(
        counters["original_tokens"],
        counters["representation_tokens"]
    );
    assert_eq!(counters["tokens_removed"], 0);
}
#[test]
fn file_batch_completion_refreshes_heartbeat_before_the_file_commits() {
    use std::{sync::mpsc, time::Duration};
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        format!("fn alpha() {{ let text = \"{}\"; }}", "abcde".repeat(1000)),
    )
    .unwrap();
    let project = dir.path().to_path_buf();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (send, events) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::operation::begin(flag);
        crate::progress::sink(Some(Arc::new(move |value| {
            let _ = send.send(value);
        })));
        let mut guard = crate::progress::Guard::start(&project, None).unwrap();
        let mut db = CacheDb::open(&project).unwrap();
        crate::service::auto_sync(&mut db, &project).unwrap();
        let mut calls = 0;
        let mut model = Mock {
            hook: Some(Box::new(move || {
                calls += 1;
                if calls == 1 {
                    std::thread::sleep(Duration::from_millis(2100));
                }
                if calls == 2 {
                    blocked.recv_timeout(Duration::from_secs(10)).unwrap();
                }
            })),
            ..Default::default()
        };
        assert!(index_with(&mut db, &project, false, &mut model).is_err());
        guard.finish("cancelled").unwrap();
        crate::progress::sink(None);
    });
    let snapshot = loop {
        let value = events.recv_timeout(Duration::from_secs(10)).unwrap();
        if value["phase"] == "inference" && value["embedded_chunks"].as_u64().unwrap_or(0) >= 4 {
            break value;
        }
    };
    cancel.store(true, Ordering::Relaxed);
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(snapshot["committed_chunks"], 0);
    assert!(snapshot["elapsed_ms"].as_u64().unwrap() >= 2000);
    assert!(
        snapshot["since_completion_ms"].as_u64().unwrap() < 2000,
        "batch progress must not wait for a file commit"
    );
}
