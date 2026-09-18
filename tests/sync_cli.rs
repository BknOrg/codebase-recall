//! `sync` behaviour that must hold across runs of the same cache.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

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

fn cached_files(dir: &Path) -> i64 {
    let conn = rusqlite::Connection::open(dir.join(".code-rcl").join("cache.db")).unwrap();
    conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap()
}

#[test]
fn language_filter_does_not_evict_other_languages() {
    let dir = project("language_filter");
    sync(&dir, &[]);
    assert_eq!(cached_files(&dir), 2);

    let filtered = sync(&dir, &["--language", "rust"]);
    assert!(filtered.contains("-0)"), "filtered sync removed files: {filtered}");
    assert_eq!(cached_files(&dir), 2, "the python file left the cache");

    let again = sync(&dir, &[]);
    assert!(again.contains("+0 ~0 =2 -0"), "unfiltered sync had to re-add: {again}");
}

#[test]
fn deleted_file_is_still_removed_under_a_filter() {
    let dir = project("deleted_under_filter");
    sync(&dir, &[]);
    std::fs::remove_file(dir.join("lib.rs")).unwrap();

    let out = sync(&dir, &["--language", "rust"]);
    assert!(out.contains("-1)"), "a really deleted file should be dropped: {out}");
    assert_eq!(cached_files(&dir), 1);
}
