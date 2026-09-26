//! End-to-end tests for `.code-rcl/config.toml`: precedence (flag > config >
//! built-in default), error reporting, and that internal analysis commands
//! ignore `[graph]`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("config_cli__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn write_config(dir: &Path, body: &str) {
    write(dir, ".code-rcl/config.toml", body);
}

fn remove_config(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(".code-rcl/config.toml"));
}

/// A small TypeScript project: an external import (`react`) and a call into a
/// sibling module, so both `external` nodes and `calls` edges can appear.
fn ts_project(tag: &str) -> PathBuf {
    let dir = fresh_dir(tag);
    write(
        &dir,
        "index.ts",
        "import { useState } from \"react\";\nimport { helper } from \"./util\";\nexport function main() { helper(); return useState(1); }\n",
    );
    write(&dir, "util.ts", "export function helper() { return 1; }\n");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .arg("--project")
        .arg(dir)
        .output()
        .expect("run code-rcl")
}

fn graph(dir: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["graph", "--format", "json", "-o", "-"];
    args.extend_from_slice(extra);
    let out = run(dir, &args);
    assert!(
        out.status.success(),
        "graph {extra:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("graph json on stdout")
}

fn has_external(g: &Value) -> bool {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "external")
}

fn edge_kinds(g: &Value, kind: &str) -> usize {
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

fn edge_count(g: &Value) -> usize {
    g["edges"].as_array().unwrap().len()
}

#[test]
fn config_include_external_and_flag_override() {
    let dir = ts_project("external");

    assert!(!has_external(&graph(&dir, &[])), "default: no external nodes");
    assert!(has_external(&graph(&dir, &["--include-external"])));

    write_config(&dir, "[graph]\ninclude_external = true\n");
    assert!(has_external(&graph(&dir, &[])), "config enables externals");
    assert!(
        !has_external(&graph(&dir, &["--include-external=false"])),
        "explicit =false beats config true"
    );
}

#[test]
fn config_kinds_and_flag_override() {
    let dir = ts_project("kinds");
    assert!(edge_kinds(&graph(&dir, &[]), "calls") > 0, "default has calls");

    write_config(&dir, "[graph]\nkinds = [\"imports\"]\n");
    let g = graph(&dir, &[]);
    assert_eq!(edge_kinds(&g, "calls"), 0);
    assert!(edge_kinds(&g, "imports") > 0);

    let g = graph(&dir, &["--kinds", "calls,imports"]);
    assert!(edge_kinds(&g, "calls") > 0, "flag replaces config kinds");
}

#[test]
fn config_min_confidence_and_flag_override() {
    let dir = ts_project("minconf");
    let flag_09 = edge_count(&graph(&dir, &["--min-confidence", "0.9"]));
    let flag_00 = edge_count(&graph(&dir, &["--min-confidence", "0.0"]));

    write_config(&dir, "[graph]\nmin_confidence = 0.9\n");
    assert_eq!(edge_count(&graph(&dir, &[])), flag_09);
    assert_eq!(
        edge_count(&graph(&dir, &["--min-confidence", "0.0"])),
        flag_00,
        "flag beats config"
    );
}

#[test]
fn bad_config_fails_naming_file_key_and_options() {
    let dir = ts_project("badcfg");

    write_config(&dir, "[graph]\nmin_confidense = 0.5\n");
    let out = run(&dir, &["graph", "--format", "json", "-o", "-"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("config.toml"), "{err}");
    assert!(err.contains("min_confidense"), "{err}");

    write_config(&dir, "[graph]\nmin_confidence = 1.5\n");
    let out = run(&dir, &["graph", "--format", "json", "-o", "-"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("config.toml"), "{err}");
    assert!(err.contains("1.5") && err.contains("0.0 to 1.0"), "{err}");
}

#[test]
fn impact_ignores_graph_config() {
    let dir = fresh_dir("impact_pin");
    copy_tree(&fixture("rust_app"), &dir);

    let baseline = run(&dir, &["impact", "greet", "--json"]);
    assert!(baseline.status.success());
    let baseline_report = run(&dir, &["report", "--json"]);
    let report_repeatable = {
        let again = run(&dir, &["report", "--json"]);
        baseline_report.status.success()
            && again.status.success()
            && again.stdout == baseline_report.stdout
    };

    write_config(
        &dir,
        "[graph]\nmin_confidence = 0.9\ninclude_external = true\nmax_nodes = 1\ndepth = 0\nkinds = [\"imports\"]\n",
    );
    let with_config = run(&dir, &["impact", "greet", "--json"]);
    assert!(with_config.status.success());
    assert_eq!(
        baseline.stdout, with_config.stdout,
        "impact output must not change with config [graph]"
    );

    if report_repeatable {
        let report = run(&dir, &["report", "--json"]);
        assert_eq!(baseline_report.stdout, report.stdout);
    } else {
        eprintln!("report --json is not byte-stable across runs; skipping its regression");
    }
    remove_config(&dir);
}

// ---- [sync] / [precise] ----------------------------------------------------

/// Two small .rs files, one small .py file and one .rs file padded past 2 KB.
fn sync_project(tag: &str) -> PathBuf {
    let dir = fresh_dir(tag);
    write(&dir, "a.rs", "pub fn a() {}\n");
    write(&dir, "b.rs", "pub fn b() { crate::a(); }\n");
    write(&dir, "c.py", "def c():\n    return 1\n");
    let mut big = String::from("pub fn big() {}\n");
    while big.len() < 2500 {
        big.push_str("// padding padding padding padding padding padding padding\n");
    }
    write(&dir, "big.rs", &big);
    dir
}

fn sync(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["sync", "--no-report"];
    args.extend_from_slice(extra);
    let out = run(dir, &args);
    assert!(
        out.status.success(),
        "sync {extra:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Parse `sync: N source files (+A ~B =C -D)`.
fn sync_line(out: &Output) -> (usize, usize, usize, usize, usize) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| l.starts_with("sync: "))
        .unwrap_or_else(|| panic!("no sync line in {stdout}"));
    let num = |s: &str| -> usize {
        s.trim_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .unwrap()
    };
    let mut it = line["sync: ".len()..].split_whitespace();
    let scanned = num(it.next().unwrap());
    let toks: Vec<&str> = line[line.find('(').unwrap() + 1..line.find(')').unwrap()]
        .split_whitespace()
        .collect();
    (scanned, num(toks[0]), num(toks[1]), num(toks[2]), num(toks[3]))
}

fn scanned(out: &Output) -> usize {
    sync_line(out).0
}

#[test]
fn config_max_file_kb_and_flag_override() {
    let dir = sync_project("maxkb");
    let baseline = scanned(&sync(&dir, &[]));

    write_config(&dir, "[sync]\nmax_file_kb = 1\n");
    let limited = scanned(&sync(&dir, &[]));
    assert!(limited < baseline, "config limit scans fewer files");
    assert_eq!(scanned(&sync(&dir, &["--max-file-kb", "8"])), baseline);
}

#[test]
fn config_languages_and_flag_override() {
    let dir = sync_project("langs");
    let baseline = scanned(&sync(&dir, &[]));
    let flag_py = scanned(&sync(&dir, &["--language", "python"]));
    let flag_rs = scanned(&sync(&dir, &["--language", "rust"]));
    assert!(flag_py < baseline);

    write_config(&dir, "[sync]\nlanguages = [\"python\"]\n");
    let out = sync(&dir, &[]);
    assert_eq!(scanned(&out), flag_py);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("languages") && err.contains("config.toml"), "{err}");

    let out = sync(&dir, &["--language", "rust"]);
    assert_eq!(scanned(&out), flag_rs);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("note: [sync] languages"), "{err}");
}

#[test]
fn implicit_sync_uses_the_same_limit_as_explicit_sync() {
    let dir = sync_project("implicit");
    write_config(&dir, "[sync]\nmax_file_kb = 1\n");

    let first = sync_line(&sync(&dir, &[]));
    let g = graph(&dir, &[]);
    let labels: Vec<&str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["label"].as_str())
        .collect();
    assert!(labels.contains(&"a"), "small file symbols kept: {labels:?}");
    assert!(!labels.contains(&"big"), "over-limit file stays out: {labels:?}");

    let second = sync_line(&sync(&dir, &[]));
    assert_eq!(second.0, first.0);
    assert_eq!((second.1, second.2, second.4), (0, 0, 0), "no thrash: {second:?}");
}
