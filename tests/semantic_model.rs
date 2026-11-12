//! Public semantic-model contract: fixtures are independently labelled source facts.
use serde_json::Value;
use std::{fs, path::Path, process::Command};
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");
fn invoke(project: &Path, args: &[&str]) -> Value {
    let result = Command::new(BIN)
        .args(args)
        .arg("--project")
        .arg(project)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["schema_version"], 2);
    value["data"].clone()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic_symbols"))
            .unwrap()
    {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    dir
}
fn graph(project: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["graph", "--format", "json", "-o", "-", "--max-nodes", "0"];
    args.extend(extra);
    invoke(project, &args)
}
fn nodes(g: &Value) -> &Vec<Value> {
    g["nodes"].as_array().unwrap()
}
#[test]
fn all_language_families_expose_canonical_kinds_and_owner_facts() {
    let dir = fixture();
    let g = graph(dir.path(), &[]);
    assert_eq!(g["version"], 3);
    assert!(g["resolution_summary"]["eligible"].as_u64().unwrap() > 0);
    let expected: Value =
        serde_json::from_slice(&fs::read(dir.path().join("expected-symbols.json")).unwrap())
            .unwrap();
    for row in expected.as_array().unwrap() {
        assert!(
            nodes(&g)
                .iter()
                .any(|n| n["path"] == row[0] && n["label"] == row[1] && n["kind"] == row[2]),
            "missing {row}"
        );
    }
    let ids: std::collections::HashSet<_> = nodes(&g)
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids.len(),
        nodes(&g).len(),
        "same-line declarations must not collide"
    );
    let extension = nodes(&g).iter().find(|n| n["label"] == "decorate").unwrap();
    assert_eq!(extension["semantic"]["extension_receiver"], "String");
    assert_eq!(extension["semantic"]["parameters"][0]["variadic"], true);
    let defaulted = nodes(&g)
        .iter()
        .find(|n| n["path"] == "symbols.kt" && n["label"] == "run")
        .unwrap();
    assert_eq!(defaulted["semantic"]["parameters"][0]["optional"], true);
    let unknown = nodes(&g).iter().find(|n| n["label"] == "dynamic").unwrap();
    assert!(unknown["semantic"]["modifiers"]["is_static"].is_null());
}

#[test]
fn tied_candidates_are_bounded_deterministic_and_never_high_confidence() {
    let dir = tempfile::tempdir().unwrap();
    let source = "class C { int pick(A x){return 0;} int pick(B x){return 0;} int pick(D x){return 0;} int pick(E x){return 0;} int pick(F x){return 0;} int pick(G x){return 0;} int start(){return pick(unknown());} }";
    fs::write(dir.path().join("C.java"), source).unwrap();
    let first = graph(dir.path(), &[]);
    let second = graph(dir.path(), &["--no-sync"]);
    let selected = |g: &Value| {
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                e["resolution"]["alternatives"]
                    .as_array()
                    .is_some_and(|a| a.len() == 3)
            })
            .unwrap()
            .clone()
    };
    let first = selected(&first);
    let second = selected(&second);
    assert_eq!(first["target"], second["target"]);
    assert_eq!(
        first["resolution"]["alternatives"],
        second["resolution"]["alternatives"]
    );
    assert_eq!(first["resolution"]["omitted_alternatives"], 2);
    assert!(first["confidence"].as_f64().unwrap() <= 0.350001);
}

#[test]
fn nested_owners_and_implicit_fields_keep_lexical_identity() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Outer.java"),
        "package p; class Outer { class Inner { int value; int read(){return value;} } }",
    )
    .unwrap();
    let g = graph(dir.path(), &["--kinds", "calls,references,contains"]);
    let read = nodes(&g).iter().find(|n| n["label"] == "read").unwrap();
    assert_eq!(read["semantic"]["owner"], "p::Outer::Inner");
    let field = nodes(&g).iter().find(|n| n["label"] == "value").unwrap();
    assert!(
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == read["id"]
                && e["target"] == field["id"]
                && e["kind"] == "references")
    );
}
#[test]
fn overload_evidence_ambiguity_confidence_and_consumers_agree() {
    let dir = fixture();
    let g = graph(dir.path(), &[]);
    let by_id: std::collections::HashMap<_, _> = nodes(&g)
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let edges = g["edges"].as_array().unwrap();
    let call = |name: &str| {
        edges
            .iter()
            .find(|e| e["kind"] == "calls" && by_id[e["source"].as_str().unwrap()]["label"] == name)
            .unwrap()
    };
    let certain = call("certain");
    let target = by_id[certain["target"].as_str().unwrap()];
    assert_eq!(target["semantic"]["parameters"][0]["declared_type"], "int");
    assert_eq!(certain["resolution"]["ambiguous"], false);
    let ambiguous = call("uncertain");
    assert_eq!(ambiguous["resolution"]["ambiguous"], true);
    assert!(ambiguous["confidence"].as_f64().unwrap() <= 0.350001);
    assert_eq!(
        ambiguous["resolution"]["alternatives"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let strict = graph(dir.path(), &["--min-confidence", "0.5"]);
    assert!(
        !strict["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == ambiguous["source"] && e["target"] == ambiguous["target"])
    );
    let search = invoke(
        dir.path(),
        &[
            "search",
            "choose",
            "--json",
            "--no-sync",
            "--no-grep",
            "--no-fuzzy",
        ],
    );
    assert_eq!(search["exact_symbols"].as_array().unwrap().len(), 2);
    assert!(search["exact_symbols"][0]["semantic"]["qualified_name"].is_string());
    let explained = invoke(
        dir.path(),
        &[
            "explain",
            target["id"].as_str().unwrap(),
            "--json",
            "--no-sync",
        ],
    );
    assert_eq!(
        explained[0]["semantic"]["parameters"][0]["declared_type"],
        "int"
    );
}
#[test]
fn structural_edges_are_opt_in_and_incremental_matches_full() {
    let dir = fixture();
    let ordinary = graph(dir.path(), &[]);
    assert!(
        !ordinary["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "inherits" || e["kind"] == "overrides")
    );
    let first = graph(dir.path(), &["--kinds", "inherits,overrides,contains"]);
    for kind in ["inherits", "overrides"] {
        assert!(
            first["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == kind),
            "missing {kind}"
        );
    }
    let second = graph(
        dir.path(),
        &["--kinds", "inherits,overrides,contains", "--no-sync"],
    );
    let canonical = |g: &Value| {
        let mut result: Vec<_> = g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["source"].to_string(),
                    e["target"].to_string(),
                    e["kind"].to_string(),
                )
            })
            .collect();
        result.sort();
        result
    };
    assert_eq!(canonical(&first), canonical(&second));
    fs::write(dir.path().join("extra.ts"), "export class Extra {}\n").unwrap();
    let incremental = graph(dir.path(), &["--kinds", "inherits,overrides,contains"]);
    let clean = fixture();
    fs::write(clean.path().join("extra.ts"), "export class Extra {}\n").unwrap();
    let full = graph(clean.path(), &["--kinds", "inherits,overrides,contains"]);
    assert_eq!(canonical(&incremental), canonical(&full));
}
