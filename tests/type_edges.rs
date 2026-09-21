//! Edges that come from type positions rather than from calls.
//!
//! A struct used only as a parameter, field, return or enum-payload type used
//! to reach the graph with no edges at all, so `impact` reported it as dead
//! code. These tests pin the `references` edges that fixed that, plus the
//! `implements` edge tying a type to the trait it implements.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn workdir(tag: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("type_edges_app");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("type_edges__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    dir
}

/// `code-rcl graph --format json` over the fixture, with every edge kind on.
fn graph_json(work: &Path) -> Value {
    let out = work.join("graph.json");
    let result = Command::new(BIN)
        .args(["graph", "--project"])
        .arg(work)
        .args(["--format", "json", "--kinds"])
        .arg("imports,calls,contains,references,implements")
        .arg("-o")
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

/// Every `(source label, target label)` pair carried by edges of `kind`.
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
            let s = label.get(e["source"].as_str()?)?;
            let t = label.get(e["target"].as_str()?)?;
            Some((s.to_string(), t.to_string()))
        })
        .collect()
}

#[test]
fn parameter_field_return_and_payload_types_become_reference_edges() {
    let work = workdir("references");
    let graph = graph_json(&work);
    let refs = edges_of_kind(&graph, "references");

    let has = |from: &str, to: &str| refs.contains(&(from.to_string(), to.to_string()));

    assert!(
        has("execute", "RunArgs"),
        "a parameter type must tie the function to the struct; got {refs:?}"
    );
    assert!(
        has("execute", "RunError"),
        "a return type must tie the function to the error type; got {refs:?}"
    );
    assert!(
        has("RunArgs", "Mode"),
        "a struct field must tie the struct to its field type; got {refs:?}"
    );
}

#[test]
fn a_generic_placeholder_does_not_reference_a_same_named_type() {
    let work = workdir("generics");
    let graph = graph_json(&work);
    let refs = edges_of_kind(&graph, "references");

    // `passthrough<Mode>` declares its own `Mode`; the project enum is unrelated.
    assert!(
        !refs.contains(&("passthrough".to_string(), "Mode".to_string())),
        "generic parameter leaked an edge to the project type: {refs:?}"
    );
}

#[test]
fn a_std_type_name_does_not_link_to_an_unrelated_project_type() {
    // `unrelated.rs` takes a `&Command` meaning `std::process::Command`, and
    // never imports `config.rs` where a project enum of that name lives.
    // Linking them made every common type name a false hub once type
    // references were added.
    let work = workdir("std_collision");
    let graph = graph_json(&work);
    let refs = edges_of_kind(&graph, "references");

    assert!(
        !refs.contains(&("spawn_tool".to_string(), "Command".to_string())),
        "a std type name resolved to the project's own type; got {refs:?}"
    );
    // The genuine same-file use must survive: `Command::Run(RunArgs)`.
    assert!(
        refs.contains(&("Command".to_string(), "RunArgs".to_string())),
        "the real payload edge regressed; got {refs:?}"
    );
}

#[test]
fn impl_trait_for_type_becomes_an_implements_edge() {
    let work = workdir("implements");
    let graph = graph_json(&work);
    let implements = edges_of_kind(&graph, "implements");

    assert!(
        implements.contains(&("LocalRunner".to_string(), "Runner".to_string())),
        "expected LocalRunner -> Runner; got {implements:?}"
    );
}

#[test]
fn a_type_only_struct_is_no_longer_reported_as_dead_code() {
    let work = workdir("impact");
    let result = Command::new(BIN)
        .args(["impact", "RunArgs", "--project"])
        .arg(&work)
        .arg("--direction")
        .arg("reverse")
        .output()
        .expect("failed to run code-rcl impact");
    assert!(
        result.status.success(),
        "impact failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("execute"),
        "impact should name the function that takes RunArgs:\n{stdout}"
    );
}
