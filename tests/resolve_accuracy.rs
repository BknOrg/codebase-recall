//! Accuracy harness for the layered reference resolver.
//!
//! Runs `code-rcl graph` on a fixture, then checks the emitted `calls` edges
//! against a hand-labelled `expected-edges.json` (`must_have` / `must_not_have`
//! tuples of `[from_path, from_name, to_path, to_name]`). Prints a small
//! precision/recall table; run with `--nocapture` to see it.

use std::collections::HashMap;
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

fn workdir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("acc__{name}"));
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

fn graph_json(work: &Path) -> Value {
    let out = work.join("graph.json");
    let status = Command::new(BIN)
        .arg("graph")
        .arg("--project")
        .arg(work)
        .args(["--format", "json", "-o"])
        .arg(&out)
        .status()
        .expect("run code-rcl graph");
    assert!(status.success(), "graph command failed");
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

/// `(from_path, from_name, to_path, to_name)` for every `calls` edge.
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

fn tuples(v: &Value, key: &str) -> Vec<(String, String, String, String)> {
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

#[test]
fn resolve_app_accuracy() {
    let name = "resolve_app";
    let work = workdir(name);
    let g = graph_json(&work);
    let edges = call_edges(&g);
    let expected: Value =
        serde_json::from_slice(&std::fs::read(fixture(name).join("expected-edges.json")).unwrap())
            .unwrap();

    let present = |q: &(String, String, String, String)| {
        edges
            .iter()
            .any(|(fp, fn_, tp, tn, _)| fp == &q.0 && fn_ == &q.1 && tp == &q.2 && tn == &q.3)
    };

    let must_have = tuples(&expected, "must_have");
    let must_not = tuples(&expected, "must_not_have");

    let hits = must_have.iter().filter(|q| present(q)).count();
    let violations: Vec<_> = must_not.iter().filter(|q| present(q)).collect();

    println!("\n=== {name}: resolved call edges ===");
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
        assert!(present(q), "missing required call edge: {q:?}");
    }
    for q in &violations {
        panic!("forbidden call edge present: {q:?}");
    }
}
