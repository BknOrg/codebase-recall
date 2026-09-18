use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .arg("--project")
        .arg(dir)
        .output()
        .expect("failed to run code-rcl")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn report_lists_subsystems_bridges_and_questions() {
    let work = workdir("two_modules", "report_md");

    let out = run(&work, &["report"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let md = stdout(&out);
    for needle in [
        "# Code Report: two_modules__report_md",
        "## Subsystems",
        "### 1.",
        "### 2.",
        "(3 files)",
        "## Bridges",
        "`invoice.rs`",
        "## Suggested questions",
        "code-rcl impact --diff",
    ] {
        assert!(md.contains(needle), "missing {needle:?} in:\n{md}");
    }

    // The report must not churn: same input, same bytes.
    assert_eq!(md, stdout(&run(&work, &["report"])));
}

#[test]
fn report_json_is_structured() {
    let work = workdir("two_modules", "report_json");

    let out = run(&work, &["report", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");

    assert_eq!(v["total_files"], 6);
    assert_eq!(v["subsystems"].as_array().unwrap().len(), 2);
    assert_eq!(v["unclustered_files"], 0);
    for s in v["subsystems"].as_array().unwrap() {
        assert_eq!(s["files"], 3);
        assert!(s["cohesion"].as_f64().unwrap() > 0.5, "{s}");
    }
    assert_eq!(v["bridges"][0]["file"], "invoice.rs");
    assert!(!v["questions"].as_array().unwrap().is_empty());
}

#[test]
fn report_output_gets_an_extension() {
    let work = workdir("two_modules", "report_out");
    let stem = work.join("overview");

    let out = Command::new(BIN)
        .args(["report", "-o"])
        .arg(&stem)
        .arg("--project")
        .arg(&work)
        .output()
        .unwrap();
    assert!(out.status.success());
    let written = std::fs::read_to_string(work.join("overview.md")).expect("overview.md written");
    assert!(written.contains("## Subsystems"));
    assert!(stdout(&out).is_empty(), "writing to a file should not also print it");
}

#[test]
fn sync_keeps_the_report_fresh_and_only_when_needed() {
    let work = workdir("two_modules", "report_sync");
    let report = work.join(".code-rcl").join("REPORT.md");

    // First sync writes the report.
    let out = run(&work, &["sync"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout(&out).contains("report: refreshed"), "stdout: {}", stdout(&out));
    let first = std::fs::read_to_string(&report).expect("REPORT.md written by sync");
    assert!(first.contains("## Subsystems"));

    // Nothing changed: the file is left alone and sync says nothing about it.
    let out = run(&work, &["sync"]);
    assert!(out.status.success());
    assert!(!stdout(&out).contains("report: refreshed"), "stdout: {}", stdout(&out));
    assert_eq!(std::fs::read_to_string(&report).unwrap(), first);

    // A source change that alters the structure refreshes it again.
    std::fs::write(
        work.join("tax.rs"),
        "use crate::label;\n\npub fn apply(value: u32) -> u32 {\n    label::print();\n    value\n}\n",
    )
    .unwrap();
    let out = run(&work, &["sync"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("report: refreshed"), "stdout: {}", stdout(&out));
    assert_ne!(std::fs::read_to_string(&report).unwrap(), first);
}

#[test]
fn sync_no_report_skips_the_report() {
    let work = workdir("two_modules", "report_skip");

    let out = run(&work, &["sync", "--no-report"]);
    assert!(out.status.success());
    assert!(!work.join(".code-rcl").join("REPORT.md").exists());
    assert!(!stdout(&out).contains("report:"));
}
