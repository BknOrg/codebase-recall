//! Accuracy harness for the layered reference resolver.
//!
//! Runs `code-rcl graph` on a fixture, then checks the emitted `calls` edges
//! against a hand-labelled `expected-edges.json` (`must_have` / `must_not_have`
//! tuples of `[from_path, from_name, to_path, to_name]`). Prints a small
//! precision/recall table; run with `--nocapture` to see it.
//!
//! Fixtures may also carry `precise_must_have` / `precise_must_not_have`: the
//! edges only a real compiler frontend gets right. Those run under `--precise`
//! against actual language servers, so they are `#[ignore]`d and additionally
//! gated behind `CODE_RCL_TEST_PRECISE=1`:
//!
//! ```text
//! CODE_RCL_TEST_PRECISE=1 cargo test --test resolve_accuracy -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

/// Set this to opt in to the tests that need language servers installed.
const PRECISE_ENV: &str = "CODE_RCL_TEST_PRECISE";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn workdir(name: &str, suffix: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("acc__{name}{suffix}"));
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

fn graph_json(work: &Path, precise: bool) -> Value {
    let out = work.join("graph.json");
    let mut command = Command::new(BIN);
    command
        .arg("graph")
        .arg("--project")
        .arg(work)
        .args(["--format", "json", "-o"])
        .arg(&out);
    if precise {
        command.arg("--precise");
    }
    let result = command.output().expect("run code-rcl graph");
    let stderr = String::from_utf8_lossy(&result.stderr);

    // A missing server is a setup problem, not a resolver bug — say so plainly
    // rather than letting the edge assertions fail with no explanation.
    if precise && stderr.contains("no language server found") {
        panic!("this test needs a language server installed:\n{stderr}");
    }
    assert!(
        result.status.success(),
        "graph command failed:\n{stderr}{}",
        String::from_utf8_lossy(&result.stdout)
    );
    if precise {
        print!("{stderr}");
    }
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

/// `(from_path, from_name, to_path, to_name, confidence)` for every `calls` edge.
fn call_edges(g: &Value) -> Vec<(String, String, String, String, f64)> {
    let nid: HashMap<&str, &Value> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let field = |n: &Value, k: &str| n.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_string();

    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "calls")
        .filter_map(|e| {
            let s = nid.get(e["source"].as_str()?)?;
            let t = nid.get(e["target"].as_str()?)?;
            Some((
                field(s, "path"),
                field(s, "label"),
                field(t, "path"),
                field(t, "label"),
                e["confidence"].as_f64().unwrap_or(0.0),
            ))
        })
        .collect()
}

type Edge = (String, String, String, String);

fn tuples(v: &Value, key: &str) -> Vec<Edge> {
    v[key]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| {
                    let g = |i: usize| t[i].as_str().unwrap().to_string();
                    (g(0), g(1), g(2), g(3))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Render the graph for `name` and hold it to the fixture's expectations.
///
/// In precise mode the `precise_*` expectations are folded in: those edges are
/// exactly the ones the heuristics get wrong, so they double as proof that
/// `--precise` changed the answer rather than just agreeing with it.
fn check_accuracy(name: &str, precise: bool) {
    let mode = if precise { "precise" } else { "heuristic" };
    let work = workdir(name, if precise { "__precise" } else { "" });
    let g = graph_json(&work, precise);
    let edges = call_edges(&g);
    let expected: Value =
        serde_json::from_slice(&std::fs::read(fixture(name).join("expected-edges.json")).unwrap())
            .unwrap();

    let present = |q: &Edge| {
        edges
            .iter()
            .any(|(fp, fn_, tp, tn, _)| fp == &q.0 && fn_ == &q.1 && tp == &q.2 && tn == &q.3)
    };

    let mut must_have = tuples(&expected, "must_have");
    let mut must_not = tuples(&expected, "must_not_have");
    if precise {
        must_have.extend(tuples(&expected, "precise_must_have"));
        must_not.extend(tuples(&expected, "precise_must_not_have"));
    }

    let hits = must_have.iter().filter(|q| present(q)).count();
    let violations: Vec<_> = must_not.iter().filter(|q| present(q)).collect();

    println!("\n=== {name} [{mode}]: resolved call edges ===");
    for (fp, fn_, tp, tn, c) in &edges {
        println!("  {fp}::{fn_}  ->  {tp}::{tn}   ({c:.2})");
    }
    let recall = hits as f64 / must_have.len().max(1) as f64;
    println!(
        "\n  recall (must_have found)   : {hits}/{}  ({:.0}%)",
        must_have.len(),
        recall * 100.0
    );
    println!(
        "  precision (must_not clean) : {}/{}  ({} violation(s))",
        must_not.len() - violations.len(),
        must_not.len(),
        violations.len()
    );

    for q in &must_have {
        assert!(present(q), "[{mode}] missing required call edge: {q:?}");
    }
    for q in &violations {
        panic!("[{mode}] forbidden call edge present: {q:?}");
    }
}

/// `false` (after printing why) when the language-server tests are not opted in.
fn precise_opted_in(servers: &str) -> bool {
    if std::env::var(PRECISE_ENV).is_ok() {
        return true;
    }
    println!(
        "skipped: set {PRECISE_ENV}=1 to run this, and make sure {servers} is installed and on PATH"
    );
    false
}

#[test]
fn resolve_app_accuracy() {
    check_accuracy("resolve_app", false);
}

/// The heuristics are expected to miss here — this pins down what they do get
/// right, so the precise run below has a baseline to improve on.
#[test]
fn rust_precise_app_heuristic_baseline() {
    check_accuracy("rust_precise_app", false);
}

#[test]
fn python_precise_app_heuristic_baseline() {
    check_accuracy("python_precise_app", false);
}

#[test]
#[ignore = "needs rust-analyzer installed; opt in with CODE_RCL_TEST_PRECISE=1"]
fn rust_precise_app_accuracy() {
    if precise_opted_in("rust-analyzer") {
        check_accuracy("rust_precise_app", true);
    }
}

#[test]
#[ignore = "needs pyright installed; opt in with CODE_RCL_TEST_PRECISE=1"]
fn python_precise_app_accuracy() {
    if precise_opted_in("pyright-langserver") {
        check_accuracy("python_precise_app", true);
    }
}
