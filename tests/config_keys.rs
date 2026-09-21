//! Config keys crossing the language boundary.
//!
//! `settings.get_string("runner.mode")` in Rust and `[runner] mode = ...` in
//! TOML are the same setting. Indexing both under one spelling is what lets a
//! single `search` show where a key is read and where it is defined.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn workdir(tag: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("config_keys_app");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("config_keys__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    dir
}

fn search(work: &Path, query: &str) -> String {
    let out = Command::new(BIN)
        .args(["search", query, "--project"])
        .arg(work)
        .output()
        .expect("failed to run code-rcl search");
    assert!(
        out.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_key_is_found_where_it_is_read_and_where_it_is_defined() {
    let work = workdir("both_sides");
    let out = search(&work, "runner.mode");

    assert!(
        out.contains("main.rs"),
        "the code that reads the key is missing:\n{out}"
    );
    assert!(
        out.contains("settings.toml"),
        "the TOML that defines the key is missing:\n{out}"
    );
}

#[test]
fn an_inline_comment_is_not_part_of_the_key() {
    let work = workdir("comments");
    let out = search(&work, "runner.jobs");
    assert!(
        out.contains("settings.toml"),
        "expected the jobs key:\n{out}"
    );
    assert!(
        !out.contains("inline comment"),
        "comment text leaked into the index:\n{out}"
    );
}

#[test]
fn a_config_file_adds_no_nodes_to_the_dependency_graph() {
    let work = workdir("graph");
    let out = work.join("graph.json");
    let result = Command::new(BIN)
        .args(["graph", "--project"])
        .arg(&work)
        .args(["--format", "json", "-o"])
        .arg(&out)
        .output()
        .expect("failed to run code-rcl graph");
    assert!(result.status.success());

    let graph: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let has_toml = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["label"].as_str().is_some_and(|l| l.ends_with(".toml")));
    assert!(
        !has_toml,
        "config files are indexed for search only; they must not enter the graph"
    );
}
