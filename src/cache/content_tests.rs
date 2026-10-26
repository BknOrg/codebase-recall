#[cfg(test)]
mod tests {
    use crate::cache::CacheDb;
    use bkndb::relational::Query;
    use std::{
        fs,
        sync::{Arc, atomic::AtomicBool},
    };
    fn start() {
        crate::operation::begin(Arc::new(AtomicBool::new(false)));
    }
    fn fixture() -> (tempfile::TempDir, CacheDb) {
        start();
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "/// reconnect surviving neighbors\npub fn cleanup() {}\nfn private_cleanup() {}\n",
        )
        .unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        (dir, db)
    }
    #[test]
    fn content_tracks_edits_renames_deletions_and_restart_without_embeddings() {
        let (dir, mut db) = fixture();
        let hits = db
            .content_candidates(
                "reconnect neighbors",
                |d| d.kind == "function" && d.is_exported,
                10,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.name, "cleanup");
        assert!(db.meta_get("embedding_config").unwrap().is_none());
        fs::write(
            dir.path().join("lib.rs"),
            "/// restore committed backup\npub fn restore() {}\n",
        )
        .unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert!(
            db.content_candidates("reconnect neighbors", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.content_candidates("committed backup", |d| d.kind == "function", 10)
                .unwrap()[0]
                .0
                .name,
            "restore"
        );
        fs::rename(dir.path().join("lib.rs"), dir.path().join("restore.rs")).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        drop(db);
        let mut db = CacheDb::open(dir.path()).unwrap();
        assert!(
            db.content_candidates("backup", |_| true, 10)
                .unwrap()
                .iter()
                .all(|(d, _)| d.path == "restore.rs")
        );
        fs::remove_file(dir.path().join("restore.rs")).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert!(
            db.content_candidates("backup", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        for table in [
            &db.tables.search_documents,
            &db.tables.search_postings,
            &db.tables.search_files,
        ] {
            assert!(db.rows(table, &Query::new()).unwrap().is_empty());
        }
    }
    #[test]
    fn documentation_survives_blank_lines_and_content_backfill_preserves_embedding_state() {
        let (dir, mut db) = fixture();
        let source = "/// commit only after the closure succeeds\n\n pub fn execute() {}\n\nfn unrelated() {}\n";
        fs::write(dir.path().join("lib.rs"), source).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        let hits = db
            .content_candidates("closure succeeds", |d| d.kind == "function", 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.name, "execute");
        let file = db.all_files().unwrap().remove(0);
        db.meta_set("embedding_file:lib.rs", "preserved-vector-marker")
            .unwrap();
        // A versioned content-index backfill must not replace AST or embeddings.
        let ids = db
            .all_symbols()
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
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
        let stats = crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert_eq!(stats.content_backfilled, 1);
        assert_eq!(
            db.meta_get("embedding_file:lib.rs").unwrap().as_deref(),
            Some("preserved-vector-marker")
        );
        assert_eq!(
            db.all_symbols()
                .unwrap()
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(db.source_snapshot(&file).unwrap(), source);
    }
    #[test]
    fn migration_from_eleven_preserves_source_symbols_and_embedding_markers_then_backfills() {
        let (dir, db) = fixture();
        let files = db.all_files().unwrap();
        let symbols = db
            .all_symbols()
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let before = db.source_snapshot(&files[0]).unwrap();
        db.meta_set("embedding_file:lib.rs", "preserved-vector-marker")
            .unwrap();
        db.db
            .write_tx(|b| {
                let mut rel = b.relational();
                for name in [
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
        db.meta_set("schema_version", "11").unwrap();
        drop(db);
        let mut db = CacheDb::open(dir.path()).unwrap();
        assert_eq!(
            db.meta_get("schema_version").unwrap().as_deref(),
            Some("12")
        );
        assert_eq!(
            db.all_symbols()
                .unwrap()
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            symbols
        );
        assert_eq!(db.source_snapshot(&files[0]).unwrap(), before);
        assert_eq!(
            db.meta_get("embedding_file:lib.rs").unwrap().as_deref(),
            Some("preserved-vector-marker")
        );
        assert!(
            db.content_candidates("neighbors", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        let stats = crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert_eq!(stats.changed, 0);
        assert!(
            !db.content_candidates("neighbors", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            !db.ensure_content_index(&files[0], before.as_bytes())
                .unwrap()
        );
        assert_eq!(
            db.meta_get("embedding_file:lib.rs").unwrap().as_deref(),
            Some("preserved-vector-marker")
        );
    }
    #[test]
    fn failed_analysis_commit_keeps_postings_statistics_and_source_together() {
        let (dir, mut db) = fixture();
        let before = db.all_files().unwrap()[0].clone();
        let tables = [
            &db.tables.search_documents,
            &db.tables.search_postings,
            &db.tables.search_files,
        ];
        let counts = tables
            .iter()
            .map(|t| db.rows(t, &Query::new()).unwrap().len())
            .collect::<Vec<_>>();
        fs::write(
            dir.path().join("lib.rs"),
            "pub fn replacement_feature() {}\n",
        )
        .unwrap();
        crate::cache::snapshots::FAIL_ANALYSIS_COMMIT.with(|flag| flag.set(true));
        assert!(crate::service::auto_sync(&mut db, dir.path()).is_err());
        assert_eq!(db.all_files().unwrap()[0].content_hash, before.content_hash);
        assert!(
            !db.content_candidates("neighbors", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            db.content_candidates("replacement feature", |_| true, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            [
                &db.tables.search_documents,
                &db.tables.search_postings,
                &db.tables.search_files
            ]
            .iter()
            .map(|t| db.rows(t, &Query::new()).unwrap().len())
            .collect::<Vec<_>>(),
            counts
        );
        start();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        assert!(
            db.content_candidates("neighbors", |_| true, 10)
                .unwrap()
                .is_empty()
        );
    }
}
