//! `sync` behaviour that must hold across runs of the same cache.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

#[test]
fn same_metadata_does_not_hide_changed_content() {
    let dir = project("same_metadata");
    let path = dir.join("lib.rs");
    sync(&dir, &[]);
    let original = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "pub fn bravo() {}\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original)
        .unwrap();
    let result = sync(&dir, &[]);
    assert!(result.contains("~1"), "{result}");
    let db = bkndb::BknDb::open(dir.join(".code-rcl/cache.bkndb")).unwrap();
    let symbols = db
        .relational()
        .table_named("symbols")
        .unwrap()
        .select()
        .run()
        .unwrap();
    assert!(
        symbols
            .iter()
            .any(|row| row.values.get("name") == Some(&bkndb::value::PropValue::from("bravo")))
    );
}

#[test]
fn changing_a_definition_invalidates_precise_results_in_unchanged_callers() {
    use bkndb::value::PropValue;
    let dir = project("precise_invalidation");
    std::fs::write(dir.join("main.rs"), "fn main() { alpha(); }\n").unwrap();
    sync(&dir, &[]);
    {
        let db = bkndb::BknDb::open(dir.join(".code-rcl/cache.bkndb")).unwrap();
        db.write_tx(|b| {
            let mut rel = b.relational();
            let refs = rel
                .table_named("refs")?
                .find(&bkndb::relational::Query::new())?;
            for rf in refs {
                rel.table_named("refs")?.update(&rf.pk, |row| {
                    row.insert("precise_status".into(), "hit".into());
                    row.insert("precise_symbol_id".into(), PropValue::Int(999));
                })?;
            }
            let files = rel
                .table_named("files")?
                .find(&bkndb::relational::Query::new())?;
            for file in files {
                rel.table_named("files")?.update(&file.pk, |row| {
                    row.insert("precise_synced_at".into(), PropValue::Int(1));
                })?;
            }
            Ok(())
        })
        .unwrap();
    }
    std::fs::write(dir.join("lib.rs"), "pub fn bravo() {}\n").unwrap();
    sync(&dir, &[]);
    let db = bkndb::BknDb::open(dir.join(".code-rcl/cache.bkndb")).unwrap();
    let refs = db
        .relational()
        .table_named("refs")
        .unwrap()
        .select()
        .run()
        .unwrap();
    assert!(refs.iter().all(|r| {
        r.values
            .get("precise_status")
            .is_none_or(|v| v == &PropValue::Null)
    }));
    let files = db
        .relational()
        .table_named("files")
        .unwrap()
        .select()
        .run()
        .unwrap();
    assert!(files.iter().all(|r| {
        r.values
            .get("precise_synced_at")
            .is_none_or(|v| v == &PropValue::Null)
    }));
}

#[test]
fn metadata_only_changes_do_not_reparse_or_advance_generation() {
    let dir = project("metadata_only");
    sync(&dir, &[]);
    let read_generation = || {
        let db = bkndb::BknDb::open(dir.join(".code-rcl/cache.bkndb")).unwrap();
        db.relational()
            .table_named("cache_meta")
            .unwrap()
            .select()
            .where_eq("key", "generation")
            .run()
            .unwrap()[0]
            .values["value"]
            .clone()
    };
    let generation = read_generation();
    let file = std::fs::File::options()
        .write(true)
        .open(dir.join("lib.rs"))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(20))
        .unwrap();
    drop(file);
    assert!(sync(&dir, &[]).contains("+0 ~0 =2 -0"));
    assert_eq!(read_generation(), generation);
}

#[cfg(windows)]
#[test]
fn unreadable_source_fails_sync_without_evicting_the_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = project("locked_source");
    sync(&dir, &[]);
    let _locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.join("lib.rs"))
        .unwrap();
    let output = Command::new(BIN)
        .args(["sync", "--no-report", "--project"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(cached_files(&dir), 2);
}

#[test]
fn schema_rebuild_preserves_configuration_and_saved_report() {
    let dir = project("schema_rebuild");
    sync(&dir, &[]);
    let config = "[sync]\nmax_file_kb = 512\n";
    std::fs::write(dir.join(".code-rcl/config.toml"), config).unwrap();
    std::fs::write(dir.join(".code-rcl/REPORT.md"), "saved report").unwrap();
    {
        let db = bkndb::BknDb::open(dir.join(".code-rcl/cache.bkndb")).unwrap();
        db.write_tx(|b| {
            b.relational().table_named("cache_meta")?.update(
                &bkndb::value::PropValue::from("schema_version"),
                |row| {
                    row.insert("value".into(), "old".into());
                },
            )?;
            Ok(())
        })
        .unwrap();
    }
    assert!(sync(&dir, &[]).contains("+2"));
    assert_eq!(
        std::fs::read_to_string(dir.join(".code-rcl/config.toml")).unwrap(),
        config
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".code-rcl/REPORT.md")).unwrap(),
        "saved report"
    );
}

fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sync__{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(dir.join("tool.py"), "def beta():\n    pass\n").unwrap();
    dir
}

fn sync(dir: &Path, extra: &[&str]) -> String {
    let out = Command::new(BIN)
        .arg("sync")
        .arg("--project")
        .arg(dir)
        .arg("--no-report")
        .args(extra)
        .output()
        .expect("run code-rcl sync");
    assert!(
        out.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Rows in the `files` table. Opens the cache directly, so call it only while
/// no `code-rcl` process is running (bkndb locks the file exclusively).
fn cached_files(dir: &Path) -> usize {
    let db = bkndb::BknDb::open(dir.join(".code-rcl").join("cache.bkndb")).unwrap();
    db.relational()
        .table_named("files")
        .unwrap()
        .select()
        .count()
        .unwrap()
}

#[test]
fn language_filter_does_not_evict_other_languages() {
    let dir = project("language_filter");
    sync(&dir, &[]);
    assert_eq!(cached_files(&dir), 2);

    let filtered = sync(&dir, &["--language", "rust"]);
    assert!(
        filtered.contains("-0)"),
        "filtered sync removed files: {filtered}"
    );
    assert_eq!(cached_files(&dir), 2, "the python file left the cache");

    let again = sync(&dir, &[]);
    assert!(
        again.contains("+0 ~0 =2 -0"),
        "unfiltered sync had to re-add: {again}"
    );
}

#[test]
fn deleted_file_is_still_removed_under_a_filter() {
    let dir = project("deleted_under_filter");
    sync(&dir, &[]);
    std::fs::remove_file(dir.join("lib.rs")).unwrap();

    let out = sync(&dir, &["--language", "rust"]);
    assert!(
        out.contains("-1)"),
        "a really deleted file should be dropped: {out}"
    );
    assert_eq!(cached_files(&dir), 1);
}

#[test]
fn locked_cache_reports_which_process_holds_it() {
    let dir = project("locked_cache");
    sync(&dir, &[]);

    // bkndb locks the file exclusively; hold it the way a running `serve` would.
    let _held = bkndb::BknDb::open(dir.join(".code-rcl").join("cache.bkndb")).unwrap();

    let out = Command::new(BIN)
        .arg("sync")
        .arg("--project")
        .arg(&dir)
        .arg("--no-report")
        .output()
        .expect("run code-rcl sync");
    assert!(
        !out.status.success(),
        "sync must fail while the cache is locked"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("in use by another code-rcl process"),
        "unhelpful lock error: {stderr}"
    );
}
