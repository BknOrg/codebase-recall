//! End-to-end tests: run the built `code-ctx` binary against fixture projects
//! and assert on the emitted JSON graph.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn workdir(name: &str, tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&fixture(name), &dir);
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), dst).unwrap();
        }
    }
}

fn graph_json(name: &str) -> Value {
    let work = workdir(name, "graph");
    let out = work.join("graph.json");
    let status = Command::new(BIN)
        .arg("graph")
        .arg("--project")
        .arg(&work)
        .args(["--format", "json", "-o"])
        .arg(&out)
        .status()
        .expect("run code-ctx graph");
    assert!(status.success(), "graph command failed for {name}");
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

fn node_ids(g: &Value) -> Vec<String> {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect()
}

fn has_edge(g: &Value, source: &str, target: &str, kind: &str) -> bool {
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["source"] == source && e["target"] == target && e["kind"] == kind)
}

/// A `calls` edge whose target node is labeled `label`.
fn calls_into(g: &Value, label: &str) -> bool {
    let target_id: Option<&str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["label"] == label && n["kind"] != "file")
        .and_then(|n| n["id"].as_str());
    match target_id {
        Some(id) => g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "calls" && e["target"] == id),
        None => false,
    }
}

fn assert_no_externals(g: &Value) {
    let n = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "external")
        .count();
    assert_eq!(n, 0, "externals should be excluded by default");
}

#[test]
fn rust_fixture_graph() {
    let g = graph_json("rust_app");
    let ids = node_ids(&g);
    assert!(ids.contains(&"file:main.rs".to_string()));
    assert!(ids.contains(&"file:util.rs".to_string()));
    assert!(
        has_edge(&g, "file:main.rs", "file:util.rs", "imports"),
        "expected main.rs -> util.rs import edge"
    );
    assert!(calls_into(&g, "greet"), "expected a call edge into greet");
    assert_no_externals(&g);
}

#[test]
fn typescript_fixture_graph() {
    let g = graph_json("ts_app");
    let ids = node_ids(&g);
    assert!(ids.contains(&"file:index.ts".to_string()));
    assert!(ids.contains(&"file:util.ts".to_string()));
    assert!(
        has_edge(&g, "file:index.ts", "file:util.ts", "imports"),
        "expected index.ts -> util.ts import edge"
    );
    assert!(calls_into(&g, "greet"), "expected a call edge into greet");
    assert_no_externals(&g);
}

#[test]
fn python_fixture_graph() {
    let g = graph_json("py_app");
    let ids = node_ids(&g);
    assert!(ids.contains(&"file:app.py".to_string()));
    assert!(ids.contains(&"file:util.py".to_string()));
    assert!(
        has_edge(&g, "file:app.py", "file:util.py", "imports"),
        "expected app.py -> util.py import edge"
    );
    assert!(calls_into(&g, "greet"), "expected a call edge into greet");
    assert_no_externals(&g);
}

#[test]
fn directory_rollup_and_degree() {
    let g = graph_json("pkg_app");
    let ids = node_ids(&g);

    // A `dir:` roll-up node plus a contains edge down to the file it holds.
    assert!(
        ids.contains(&"dir:pkg".to_string()),
        "expected a dir: node for the pkg/ directory"
    );
    assert!(
        has_edge(&g, "dir:pkg", "file:pkg/util.py", "contains"),
        "expected dir:pkg -> file:pkg/util.py contains edge"
    );

    // Nested files carry their parent directory.
    let util = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "file:pkg/util.py")
        .expect("pkg/util.py node");
    assert_eq!(util["dir"], "pkg");

    // Every file/symbol node is annotated with a relation degree.
    assert!(
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["kind"] == "file")
            .all(|n| n["degree"].is_u64()),
        "file nodes should all have a numeric degree"
    );
}

#[test]
fn sync_is_incremental() {
    let work = workdir("rust_app", "incremental");

    let first = Command::new(BIN)
        .arg("sync")
        .arg("--project")
        .arg(&work)
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_out = String::from_utf8_lossy(&first.stdout);
    assert!(
        first_out.contains("+2"),
        "first sync should add 2 files: {first_out}"
    );

    let second = Command::new(BIN)
        .arg("sync")
        .arg("--project")
        .arg(&work)
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_out = String::from_utf8_lossy(&second.stdout);
    assert!(
        second_out.contains("+0 ~0 =2"),
        "second sync should be a no-op: {second_out}"
    );

    // Touch one file; only it should re-parse.
    let util = work.join("util.rs");
    let mut src = std::fs::read_to_string(&util).unwrap();
    src.push_str("\npub fn extra() {}\n");
    std::fs::write(&util, src).unwrap();

    let third = Command::new(BIN)
        .arg("sync")
        .arg("--project")
        .arg(&work)
        .output()
        .unwrap();
    let third_out = String::from_utf8_lossy(&third.stdout);
    assert!(
        third_out.contains("+0 ~1 =1"),
        "third sync should re-parse exactly one file: {third_out}"
    );
}

#[test]
fn vue_fixture_graph() {
    let g = graph_json("vue_app");
    assert_no_externals(&g);

    let ids = node_ids(&g);
    assert!(ids.contains(&"file:App.vue".to_string()));
    assert!(ids.contains(&"file:HeaderBar.vue".to_string()));

    // Edge from App.vue to HeaderBar.vue
    assert!(has_edge(&g, "file:App.vue", "file:HeaderBar.vue", "imports"));
}

