use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn help_commands_work_for_all_subcommands() {
    let subcommands = ["dump", "init", "sync", "graph", "serve", "impact"];
    for sub in subcommands {
        let out = Command::new(BIN)
            .args([sub, "help"])
            .output()
            .unwrap_or_else(|_| panic!("failed to run {sub} help"));
        assert!(out.status.success(), "{sub} help failed");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Usage:"),
            "{sub} help missing Usage section: {stdout}"
        );
        assert!(
            stdout.contains("Examples:"),
            "{sub} help missing Examples section: {stdout}"
        );
    }
}

#[test]
fn top_level_help_lists_all_commands() {
    let out = Command::new(BIN)
        .arg("help")
        .output()
        .expect("failed to run code-rcl help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in ["dump", "init", "sync", "graph", "serve", "impact"] {
        assert!(
            stdout.contains(sub),
            "top level help missing {sub}: {stdout}"
        );
    }
}

#[test]
fn impact_analysis_tree_and_json() {
    let work = workdir("rust_app", "impact");

    // Test ASCII tree
    let out = Command::new(BIN)
        .args(["impact", "decorate", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Impact Analysis for: decorate"));
    assert!(stdout.contains("Direct callers: 1"));
    assert!(stdout.contains("greet [calls]"));
    assert!(stdout.contains("main [calls]"));

    // Test JSON output
    let out_json = Command::new(BIN)
        .args(["impact", "decorate", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --json");
    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let val: serde_json::Value =
        serde_json::from_str(&stdout_json).expect("valid JSON expected from impact --json");
    assert_eq!(val["target_symbol"], "decorate");
    assert_eq!(val["direct_callers_count"], 1);
    assert_eq!(val["total_affected_count"], 2);
    assert_eq!(val["callers"][0]["label"], "greet");
    assert_eq!(val["callers"][0]["callers"][0]["label"], "main");
}
