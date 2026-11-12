//! Public contracts for C/C++: context, cache freshness, graph and source facts.
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .arg("--project")
        .arg(root)
        .output()
        .unwrap()
}
fn value(root: &Path, args: &[&str]) -> Value {
    let out = run(root, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema_version"], 2);
    v["data"].clone()
}
fn graph(root: &Path, extra: &[&str]) -> Value {
    let mut args = vec![
        "graph",
        "--format",
        "json",
        "-o",
        "-",
        "--min-confidence",
        "0",
        "--kinds",
        "calls,imports,references,contains,inherits,overrides",
    ];
    args.extend(extra);
    value(root, &args)
}
fn write(root: &Path, path: &str, src: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, src).unwrap();
}
fn copy_fixture(name: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for entry in fs::read_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
    {
        let entry = entry.unwrap();
        fs::copy(entry.path(), d.path().join(entry.file_name())).unwrap();
    }
    d
}
#[test]
fn categories_scopes_includes_and_unicode_are_public() {
    let d = copy_fixture("semantic_cpp_app");
    let g = graph(d.path(), &[]);
    let nodes = g["nodes"].as_array().unwrap();
    assert_eq!(g["version"], 3);
    for kind in [
        "concept",
        "destructor",
        "constructor",
        "class",
        "struct",
        "field",
        "type_alias",
        "namespace",
    ] {
        assert!(nodes.iter().any(|n| n["kind"] == kind), "missing {kind}");
    }
    assert!(!nodes.iter().any(|n| n["label"] == "local"));
    let ids: std::collections::HashSet<_> =
        nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), nodes.len());
    let helper = nodes.iter().find(|n| n["label"] == "helper").unwrap();
    assert_eq!(helper["language"], "cpp");
    assert!(
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "imports")
    );
    value(d.path(), &["search", "helper", "--json"]);
    value(
        d.path(),
        &["explain", helper["id"].as_str().unwrap(), "--json"],
    );
    value(d.path(), &["digest", "--json"]);
    value(d.path(), &["impact", "helper", "--json"]);
    value(d.path(), &["report", "--json"]);
}
#[test]
fn flags_and_header_override_changes_reparse_unchanged_sources() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "a.c",
        "#include \"api.h\"\nint a(void){return helper();}",
    );
    write(d.path(), "api.h", "int helper(void){return 1;}");
    write(d.path(), "compile_flags.txt", "-std=c17\n");
    graph(d.path(), &[]);
    write(d.path(), "compile_flags.txt", "-std=c23\n-DMODE=2\n");
    let stale = run(
        d.path(),
        &["graph", "--format", "json", "-o", "-", "--no-sync"],
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("context changed"));
    let incremental = graph(d.path(), &[]);
    fs::remove_dir_all(d.path().join(".code-rcl")).unwrap();
    value(d.path(), &["sync", "--json", "--no-report"]);
    let full = graph(d.path(), &["--no-sync"]);
    assert_eq!(incremental["nodes"], full["nodes"]);
    assert_eq!(incremental["edges"], full["edges"]);
    write(
        d.path(),
        ".code-rcl/config.toml",
        "[c_family]\nheader_language_overrides=[{pattern='*.h',language='cpp'}]\n",
    );
    assert!(
        !run(
            d.path(),
            &["graph", "--no-sync", "--format", "json", "-o", "-"]
        )
        .status
        .success()
    );
    let changed = graph(d.path(), &[]);
    assert!(
        changed["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["path"] == "api.h" && n["language"] == "cpp")
    );
}

#[test]
fn cpp_inheritance_uses_namespace_and_requires_virtual_override_evidence() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "types.cpp",
        "namespace one { struct Base { protected: int secret; public: virtual int run(int a){return a;} int plain(){return 1;} }; struct Child: Base { int run(int a) override {return a;} int plain(){return 2;} int read(){return secret;} }; } namespace two { struct Base {}; } int outsider(one::Base &v){return v.secret;}",
    );
    let g = graph(d.path(), &[]);
    let nodes = g["nodes"].as_array().unwrap();
    let by_id = |id: &Value| nodes.iter().find(|n| n["id"] == *id).unwrap();
    assert!(
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "inherits"
                && by_id(&e["target"])["semantic"]["qualified_name"] == "one::Base")
    );
    let overrides: Vec<_> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "overrides")
        .collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(by_id(&overrides[0]["source"])["label"], "run");
    let secret_refs: Vec<_> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "references" && by_id(&e["target"])["label"] == "secret")
        .collect();
    assert!(
        secret_refs
            .iter()
            .any(|e| by_id(&e["source"])["label"] == "read")
    );
    assert!(
        !secret_refs
            .iter()
            .any(|e| by_id(&e["source"])["label"] == "outsider")
    );
}
#[test]
fn precise_rejects_missing_or_incomplete_build_context_with_explicit_fallback() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "main.cpp",
        "int helper(){return 1;} int main(){return helper();}",
    );
    let out = run(
        d.path(),
        &["graph", "--precise", "--format", "json", "-o", "-"],
    );
    assert!(!out.status.success());
    let fallback = run(
        d.path(),
        &[
            "graph",
            "--precise",
            "--allow-heuristic-fallback",
            "--format",
            "json",
            "-o",
            "-",
        ],
    );
    assert!(fallback.status.success());
    let envelope: Value = serde_json::from_slice(&fallback.stdout).unwrap();
    assert_eq!(envelope["status"], "partial");
    write(d.path(), "compile_commands.json", &json!([]).to_string());
    assert!(
        !run(
            d.path(),
            &["graph", "--precise", "--format", "json", "-o", "-"]
        )
        .status
        .success()
    );
    write(d.path(), "compile_commands.json", "malformed");
    assert!(
        !run(d.path(), &["sync", "--precise", "--no-report"])
            .status
            .success()
    );
}
#[test]
#[ignore = "requires installed clangd; opt in with CODE_RCL_TEST_PRECISE=1"]
fn real_clangd_resolves_c17_c23_cpp23_and_invalidates_flags() {
    if std::env::var("CODE_RCL_TEST_PRECISE").as_deref() != Ok("1") {
        return;
    }
    for (ext, flags, source) in [
        (
            "c",
            "-std=c17",
            "int helper(void){return 1;} int main(void){return helper();}",
        ),
        (
            "c",
            "-std=c23",
            "constexpr int n=1; int helper(void){return n;} int main(void){return helper();}",
        ),
        (
            "cpp",
            "-std=c++23",
            "constexpr int helper(){if consteval {return 1;} else {return 2;}} int main(){return helper();}",
        ),
    ] {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &format!("main.{ext}"), source);
        write(d.path(), "compile_flags.txt", flags);
        let g = graph(d.path(), &["--precise"]);
        assert!(
            g["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["resolution"]["evidence"] == "lsp"
                    && e["target"].as_str().unwrap().contains("#helper@")),
            "{g}"
        );
        write(
            d.path(),
            "compile_flags.txt",
            &format!("{flags}\n-DCHANGED=1\n"),
        );
        assert!(
            !run(
                d.path(),
                &["graph", "--no-sync", "--format", "json", "-o", "-"]
            )
            .status
            .success()
        );
        graph(d.path(), &["--precise"]);
    }
}

#[test]
fn recovered_header_sync_is_unique_warm_and_consistent() {
    let dir = copy_fixture("header_duplicate_app");
    let root = dir.path();
    let cold_out = run(root, &["sync", "--json", "--no-report"]);
    assert!(
        cold_out.status.success(),
        "{}",
        String::from_utf8_lossy(&cold_out.stderr)
    );
    let cold: Value = serde_json::from_slice(&cold_out.stdout).unwrap();
    assert_eq!(cold["status"], "partial");
    assert!(
        cold["metadata"]["timings_ms"]
            .get("header_language_probe")
            .is_some()
    );
    let cold_graph = graph(root, &["--no-sync"]);
    let nodes = cold_graph["nodes"].as_array().unwrap();
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n["kind"] == "enum" && n["label"] == "Type")
            .count(),
        1
    );
    let event = nodes
        .iter()
        .find(|n| n["label"] == "Event" && n["kind"] == "class")
        .unwrap();
    assert_eq!(event["language"], "cpp");
    assert_eq!(
        event["semantic"]["language_details"]["build_context"]["header_language_source"],
        "syntax_fallback"
    );
    assert_eq!(
        event["semantic"]["language_details"]["build_context"]["header_language_clear"],
        false
    );
    let warm_out = run(root, &["sync", "--json", "--no-report"]);
    assert!(warm_out.status.success());
    let warm: Value = serde_json::from_slice(&warm_out.stdout).unwrap();
    assert_eq!(warm["data"]["unchanged"], 1);
    assert!(
        warm["metadata"]["timings_ms"]
            .get("header_language_probe")
            .is_none()
    );
    assert_eq!(warm["status"], cold["status"]);
    assert_eq!(graph(root, &["--no-sync"])["nodes"], cold_graph["nodes"]);
    write(root, "compile_flags.txt", "-Wall\n");
    write(root, "not-a-server.exe", "this file must never execute");
    let precise = Command::new(BIN)
        .args(["sync", "--precise", "--no-report", "--project"])
        .arg(root)
        .env("CODE_RCL_LSP_CLANGD", root.join("not-a-server.exe"))
        .output()
        .unwrap();
    assert!(!precise.status.success());
    assert!(
        String::from_utf8_lossy(&precise.stderr).contains("ambiguous header language"),
        "{}",
        String::from_utf8_lossy(&precise.stderr)
    );
    let src = fs::read_to_string(root.join("Event.h"))
        .unwrap()
        .replace("        Timer,", "        Timer,\n        Extra,");
    write(root, "Event.h", &src);
    let mut incremental = graph(root, &[]);
    let fresh = tempfile::tempdir().unwrap();
    write(fresh.path(), "Event.h", &src);
    write(fresh.path(), "compile_flags.txt", "-Wall\n");
    let mut full = graph(fresh.path(), &[]);
    for g in [&mut incremental, &mut full] {
        for n in g["nodes"].as_array_mut().unwrap() {
            if let Some(details) =
                n["semantic"]["language_details"]["build_context"].as_object_mut()
            {
                details.remove("fingerprint");
            }
        }
    }
    assert_eq!(incremental["nodes"], full["nodes"]);
    assert_eq!(incremental["edges"], full["edges"]);
}
