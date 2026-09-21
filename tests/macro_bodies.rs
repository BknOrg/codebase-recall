//! Edges that only exist inside a macro's arguments, and names handed to a
//! function rather than called.
//!
//! tree-sitter leaves a macro's arguments as raw tokens, so a call written
//! inside `futures::try_join!(..)` used to be invisible: the callee looked like
//! dead code and any path through it dead-ended. These tests pin the re-parse
//! that fixed it, and the guard that keeps locals out of the graph.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn workdir(tag: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("macro_body_app");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("macro_bodies__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    dir
}

fn graph_json(work: &Path) -> Value {
    let out = work.join("graph.json");
    let result = Command::new(BIN)
        .args(["graph", "--project"])
        .arg(work)
        .args(["--format", "json", "--kinds"])
        .arg("imports,calls,contains,references")
        .args(["--max-nodes", "0", "-o"])
        .arg(&out)
        .output()
        .expect("failed to run code-rcl graph");
    assert!(
        result.status.success(),
        "graph failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap()
}

fn edges_of_kind(graph: &Value, kind: &str) -> HashSet<(String, String)> {
    let label: std::collections::HashMap<&str, &str> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n["label"].as_str().unwrap()))
        .collect();
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .filter_map(|e| {
            Some((
                label.get(e["source"].as_str()?)?.to_string(),
                label.get(e["target"].as_str()?)?.to_string(),
            ))
        })
        .collect()
}

#[test]
fn a_call_inside_a_macro_becomes_an_edge() {
    let work = workdir("calls");
    let calls = edges_of_kind(&graph_json(&work), "calls");

    assert!(
        calls.contains(&("cmd_run".to_string(), "run_inner".to_string())),
        "call inside try_join! is missing; got {calls:?}"
    );
    assert!(
        calls.contains(&("tally".to_string(), "finish".to_string())),
        "call inside assert! is missing; got {calls:?}"
    );
}

#[test]
fn a_function_passed_as_a_value_becomes_an_edge() {
    let work = workdir("value");
    let refs = edges_of_kind(&graph_json(&work), "references");

    assert!(
        refs.contains(&("cmd_run".to_string(), "dispatch_handler".to_string())),
        "a handler passed as an argument must reach its definition; got {refs:?}"
    );
}

#[test]
fn a_local_passed_as_an_argument_is_not_an_edge() {
    // `tally` declares `let finish = 3` and passes it to `helper`. That must
    // not link to the `finish` function in worker.rs.
    let work = workdir("local");
    let refs = edges_of_kind(&graph_json(&work), "references");

    assert!(
        !refs.contains(&("tally".to_string(), "finish".to_string())),
        "a local shadowing a function name leaked an edge; got {refs:?}"
    );
}

#[test]
fn a_symbol_reached_only_through_a_macro_is_not_dead_code() {
    let work = workdir("impact");
    let out = Command::new(BIN)
        .args(["impact", "run_inner", "--project"])
        .arg(&work)
        .args(["--direction", "reverse"])
        .output()
        .expect("failed to run code-rcl impact");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("cmd_run"),
        "impact must name the caller hidden in the macro:\n{stdout}"
    );
}
