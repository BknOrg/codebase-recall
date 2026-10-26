#[cfg(test)]
mod tests {
    use crate::cache::{
        CacheDb,
        models::FileRow,
        snapshots::{FAIL_ANALYSIS_COMMIT, FAIL_MIGRATION_COMMIT},
    };
    use crate::{operation, service};
    use bkndb::{BknError, value::PropValue};
    use std::{
        fs,
        sync::{Arc, atomic::AtomicBool},
    };

    fn start() {
        operation::begin(Arc::new(AtomicBool::new(false)));
    }
    fn fixture() -> (tempfile::TempDir, CacheDb, FileRow) {
        start();
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "// outside symbols 日本語\r\n/// doc\r\npub fn alpha() {}\r\n",
        )
        .unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        service::auto_sync(&mut db, dir.path()).unwrap();
        let file = db.all_files().unwrap().remove(0);
        (dir, db, file)
    }

    #[test]
    fn snapshot_preserves_original_bytes_and_does_not_read_the_source_file() {
        let (dir, mut db, file) = fixture();
        let bytes = fs::read(dir.path().join("lib.rs")).unwrap();
        assert!(!db.ensure_source_snapshot(&file, &bytes).unwrap());
        fs::remove_file(dir.path().join("lib.rs")).unwrap();
        assert_eq!(db.source_snapshot(&file).unwrap().as_bytes(), bytes);
        service::auto_sync(&mut db, dir.path()).unwrap();
        assert!(db.source_snapshot(&file).is_err());
        assert!(
            db.rows(
                &db.tables.source_snapshots,
                &bkndb::relational::Query::new()
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn unchanged_sync_repairs_missing_corrupt_and_mismatched_snapshots() {
        let (dir, mut db, file) = fixture();
        let generation = db.meta_get("generation").unwrap();
        let ids: Vec<_> = db
            .symbols_for_file(file.id)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        for damage in ["missing", "hash", "content", "type"] {
            db.db
                .write_tx(|b| {
                    let mut rel = b.relational();
                    let mut table = rel.table(db.tables.source_snapshots.clone());
                    if damage == "missing" {
                        table.delete(&file.id.into())?;
                    } else {
                        table.update(&file.id.into(), |r| match damage {
                            "hash" => {
                                r.insert("source_hash".into(), "wrong".into());
                            }
                            "content" => {
                                r.insert(
                                    "content".into(),
                                    PropValue::Bytes(b"wrong bytes".to_vec()),
                                );
                            }
                            _ => {
                                r.remove("content");
                            }
                        })?;
                    }
                    Ok(())
                })
                .unwrap();
            let error = db.source_snapshot(&file).unwrap_err().to_string();
            assert!(error.contains("source sync"), "{damage}: {error}");
            start();
            let stats = service::auto_sync(&mut db, dir.path()).unwrap();
            assert_eq!((stats.changed, stats.unchanged), (0, 1));
            assert_eq!(
                db.source_snapshot(&file).unwrap(),
                fs::read_to_string(dir.path().join("lib.rs")).unwrap()
            );
            assert_eq!(db.meta_get("generation").unwrap(), generation);
            assert_eq!(
                db.symbols_for_file(file.id)
                    .unwrap()
                    .iter()
                    .map(|s| s.id)
                    .collect::<Vec<_>>(),
                ids
            );
        }
        let mut stale = file.clone();
        stale.content_hash = "stale".into();
        assert!(
            db.source_snapshot(&stale)
                .unwrap_err()
                .to_string()
                .contains("identity")
        );
        assert!(db.ensure_source_snapshot(&stale, b"new").is_err());
    }

    #[test]
    fn non_utf8_source_is_preserved_but_rejected_for_embedding() {
        let (dir, mut db, _) = fixture();
        fs::write(
            dir.path().join("lib.rs"),
            b"// invalid \xff\npub fn alpha() {}\n",
        )
        .unwrap();
        service::auto_sync(&mut db, dir.path()).unwrap();
        let file = db.all_files().unwrap().remove(0);
        let error = db.source_snapshot(&file).unwrap_err().to_string();
        assert!(error.contains("UTF-8"), "{error}");
        assert!(
            !db.ensure_source_snapshot(&file, &fs::read(dir.path().join("lib.rs")).unwrap())
                .unwrap()
        );
    }

    #[test]
    fn failed_analysis_transaction_preserves_snapshot_metadata_and_embedding_marker() {
        let (dir, mut db, file) = fixture();
        let before = db.source_snapshot(&file).unwrap();
        db.meta_set("embedding_file:lib.rs", "old-marker").unwrap();
        FAIL_ANALYSIS_COMMIT.with(|flag| flag.set(true));
        let source = "pub fn changed() {}\n";
        let parsed = crate::analysis::parse_file(crate::analysis::Language::Rust, source);
        assert!(
            db.replace_file_analysis(
                "lib.rs",
                "rust",
                blake3::hash(source.as_bytes()).to_hex().as_str(),
                None,
                Some(source.len() as i64),
                true,
                &parsed.symbols,
                &parsed.imports,
                &parsed.refs,
                &parsed.scopes,
                &parsed.bindings,
                &parsed.string_literals,
                Some(source.as_bytes())
            )
            .is_err()
        );
        assert_eq!(db.source_snapshot(&file).unwrap(), before);
        assert_eq!(db.all_files().unwrap()[0].content_hash, file.content_hash);
        assert_eq!(
            db.meta_get("embedding_file:lib.rs").unwrap().as_deref(),
            Some("old-marker")
        );
        drop(db);
        drop(dir);
    }

    #[test]
    fn migration_conflict_never_resets_existing_tables_or_bumps_version() {
        let (dir, db, file) = fixture();
        db.meta_set("schema_version", "10").unwrap();
        drop(db);
        assert!(
            CacheDb::open(dir.path())
                .err()
                .unwrap()
                .to_string()
                .contains("migrating")
        );
        let raw = bkndb::BknDb::open(dir.path().join(".code-rcl/cache.bkndb")).unwrap();
        raw.read_tx(|r| {
            let rel = r.relational();
            let version = rel
                .table_named("cache_meta")?
                .get(&"schema_version".into())?
                .unwrap();
            assert_eq!(version.values["value"], PropValue::from("10"));
            assert!(rel.table_named("files")?.get(&file.id.into())?.is_some());
            assert!(
                rel.table_named("source_snapshots")?
                    .get(&file.id.into())?
                    .is_some()
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn interrupted_migration_rolls_back_new_table_and_schema_version() {
        let (dir, db, file) = fixture();
        db.db
            .write_tx(|b| {
                let mut rel = b.relational();
                for name in [
                    "source_snapshots",
                    "search_documents",
                    "search_postings",
                    "search_files",
                    "purpose_contexts",
                    "purpose_files",
                ] {
                    rel.drop_table(name)?;
                }
                Ok(())
            })
            .unwrap();
        db.meta_set("schema_version", "10").unwrap();
        drop(db);
        FAIL_MIGRATION_COMMIT.with(|flag| flag.set(true));
        assert!(CacheDb::open(dir.path()).is_err());
        {
            let raw = bkndb::BknDb::open(dir.path().join(".code-rcl/cache.bkndb")).unwrap();
            raw.read_tx(|r| {
                let rel = r.relational();
                assert!(rel.table_schema("source_snapshots")?.is_none());
                assert_eq!(
                    rel.table_named("cache_meta")?
                        .get(&"schema_version".into())?
                        .unwrap()
                        .values["value"],
                    PropValue::from("10")
                );
                assert!(rel.table_named("files")?.get(&file.id.into())?.is_some());
                Ok(())
            })
            .unwrap();
        }
        let db = CacheDb::open(dir.path()).unwrap();
        assert_eq!(
            db.meta_get("schema_version").unwrap().as_deref(),
            Some("12")
        );
    }

    #[test]
    fn source_snapshots_obey_ignore_language_and_size_scope_without_a_model() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".code-rcl")).unwrap();
        fs::write(
            dir.path().join(".code-rcl/config.toml"),
            "[sync]\nlanguages=['rust']\nmax_file_kb=1\n",
        )
        .unwrap();
        fs::write(dir.path().join("ok.rs"), "pub fn ok() {}\n").unwrap();
        fs::write(dir.path().join("too_large.rs"), "x".repeat(2048)).unwrap();
        fs::write(dir.path().join("other.py"), "def other(): pass\n").unwrap();
        fs::write(dir.path().join(".hidden.rs"), "fn hidden() {}\n").unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        start();
        service::auto_sync(&mut db, dir.path()).unwrap();
        let rows = db
            .rows(
                &db.tables.source_snapshots,
                &bkndb::relational::Query::new(),
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(db.all_files().unwrap()[0].path, "ok.rs");
    }

    #[test]
    #[ignore = "explicit isolated corpus audit; set CODE_RCL_SNAPSHOT_AUDIT_PROJECT"]
    fn source_snapshot_corpus_migration_preserves_all_graph_rows_and_vector_bytes() {
        use bkndb::relational::Query;
        let project =
            std::path::PathBuf::from(std::env::var("CODE_RCL_SNAPSHOT_AUDIT_PROJECT").unwrap());
        let names = [
            "files",
            "symbols",
            "imports",
            "refs",
            "scopes",
            "bindings",
            "string_literals",
            "embedding_chunks",
        ];
        let before = {
            let raw = bkndb::BknDb::open(project.join(".code-rcl/cache.bkndb")).unwrap();
            raw.read_tx(|r| {
                let rel = r.relational();
                assert_eq!(
                    rel.table_named("cache_meta")?
                        .get(&"schema_version".into())?
                        .unwrap()
                        .values["value"],
                    PropValue::from("10")
                );
                names
                    .iter()
                    .map(|name| {
                        let mut rows = rel
                            .table_named(name)?
                            .find(&Query::new())?
                            .into_iter()
                            .map(|row| (format!("{:?}", row.pk), row.values))
                            .collect::<Vec<_>>();
                        rows.sort_by(|a, b| a.0.cmp(&b.0));
                        Ok(rows)
                    })
                    .collect::<std::result::Result<Vec<_>, BknError>>()
            })
            .unwrap()
        };
        start();
        let mut db = CacheDb::open(&project).unwrap();
        let stats = service::auto_sync(&mut db, &project).unwrap();
        assert_eq!(stats.changed, 0);
        for (name, expected) in names.iter().zip(&before) {
            let mut actual = db
                .db
                .read_tx(|r| r.relational().table_named(name)?.find(&Query::new()))
                .unwrap()
                .into_iter()
                .map(|row| (format!("{:?}", row.pk), row.values))
                .collect::<Vec<_>>();
            actual.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(actual, *expected, "changed table: {name}");
        }
        let files = db.all_files().unwrap();
        for file in &files {
            assert_eq!(
                db.source_snapshot(file).unwrap().as_bytes(),
                fs::read(project.join(&file.path)).unwrap()
            );
        }
        let report = serde_json::json!({"schema_before":10,"schema_after":11,"files":files.len(),"embedding_chunks":before.last().unwrap().len(),"source_bytes_verified":true,"graph_rows_and_vector_bytes_identical":true,"source_sync_unchanged":stats.unchanged});
        let output = std::env::var("CODE_RCL_SNAPSHOT_AUDIT_OUTPUT")
            .unwrap_or_else(|_| "target/source-snapshot-corpus-audit.json".into());
        fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{report}");
    }
}
