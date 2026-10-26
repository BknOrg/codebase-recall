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

fn has_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_git_repo(dir: &Path) -> bool {
    if !has_git() {
        eprintln!("skipping test: git is not installed or not in PATH");
        return false;
    }
    git(dir, &["init"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "baseline"]);
    true
}

#[test]
fn help_commands_work_for_all_subcommands() {
    let subcommands = [
        "dump", "init", "sync", "graph", "serve", "impact", "path", "explain",
    ];
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
    for sub in [
        "dump", "init", "sync", "graph", "serve", "impact", "path", "explain",
    ] {
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
    assert!(stdout.contains("IMPACT ANALYSIS: decorate"));
    assert!(stdout.contains("Direct callers : 1"));
    assert!(stdout.contains("greet"));
    assert!(stdout.contains("main"));
    assert!(stdout.contains("[calls →"));

    // Test JSON output
    let out_json = Command::new(BIN)
        .args(["impact", "decorate", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --json");
    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let parsed: serde_json::Value =
        support::from_str(&stdout_json).expect("valid JSON expected from impact --json");
    // Always an array, even for a single match — a script parsing this
    // output shouldn't have to special-case "one match" vs "several".
    let reports = parsed
        .as_array()
        .expect("impact --json must be a JSON array");
    assert_eq!(
        reports.len(),
        1,
        "expected exactly one match for 'decorate'"
    );
    let val = &reports[0];
    assert_eq!(val["target_symbol"], "decorate");
    assert_eq!(val["direct_callers_count"], 1);
    assert_eq!(val["total_affected_count"], 2);
    assert_eq!(val["callers"][0]["label"], "greet");
    assert_eq!(val["callers"][0]["callers"][0]["label"], "main");
}

#[test]
fn impact_diff_detects_changed_symbol() {
    let work = workdir("rust_app", "impact_diff_changed");
    if !init_git_repo(&work) {
        return;
    }

    let util_path = work.join("util.rs");
    let original = std::fs::read_to_string(&util_path).unwrap();
    let modified = original.replace(
        "format!(\"hello, {name}\")",
        "format!(\"hello there, {name}\")",
    );
    assert_ne!(
        original, modified,
        "fixture content did not change as expected"
    );
    std::fs::write(&util_path, modified).unwrap();

    let out = Command::new(BIN)
        .args(["impact", "--diff", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --diff");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("IMPACT ANALYSIS: decorate"),
        "stdout: {stdout}"
    );

    let out_json = Command::new(BIN)
        .args(["impact", "--diff", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --diff --json");
    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let parsed: serde_json::Value =
        support::from_str(&stdout_json).expect("valid JSON expected from impact --diff --json");
    let reports = parsed
        .as_array()
        .expect("impact --json must be a JSON array");
    assert_eq!(reports.len(), 1);
    let val = &reports[0];
    assert_eq!(val["target_symbol"], "decorate");
    assert!(val["callers"][0].get("is_exported").is_some());
    assert!(val["callers"][0].get("low_confidence").is_some());
    assert!(val["callers"][0].get("crosses_module").is_some());
    assert!(val.get("risky_affected_count").is_some());
}

#[test]
fn impact_diff_no_changes_is_friendly() {
    let work = workdir("rust_app", "impact_diff_clean");
    if !init_git_repo(&work) {
        return;
    }

    let out = Command::new(BIN)
        .args(["impact", "--diff", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --diff");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No uncommitted changes detected"),
        "stdout: {stdout}"
    );

    let out_json = Command::new(BIN)
        .args(["impact", "--diff", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact --diff --json");
    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let parsed: serde_json::Value = support::from_str(&stdout_json).expect("valid JSON expected");
    assert_eq!(parsed.as_array().unwrap().len(), 0);
}

#[test]
fn impact_requires_symbol_or_diff_flag() {
    let work = workdir("rust_app", "impact_no_target");
    if !init_git_repo(&work) {
        return;
    }

    let out = Command::new(BIN)
        .args(["impact", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("requires either a <SYMBOL> argument or --diff"),
        "stderr: {stderr}"
    );

    let out_combined = Command::new(BIN)
        .args(["impact", "decorate", "--diff", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl impact");
    assert!(!out_combined.status.success());
    let stderr_combined = String::from_utf8_lossy(&out_combined.stderr);
    assert!(
        stderr_combined.contains("cannot be combined"),
        "stderr: {stderr_combined}"
    );
}

#[test]
fn path_finds_chain_and_reports_missing_path() {
    let work = workdir("rust_app", "path_cmd");

    let out = Command::new(BIN)
        .args(["path", "main", "decorate", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl path");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        support::from_str(&String::from_utf8_lossy(&out.stdout)).expect("valid JSON");
    let report = &parsed.as_array().expect("array")[0];
    assert_eq!(report["found"], true);
    assert_eq!(report["length"], 2);
    let labels: Vec<&str> = report["hops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["main", "greet", "decorate"]);

    // Forward from a leaf back to main has no path: valid result, not an error.
    let out = Command::new(BIN)
        .args(["path", "decorate", "main", "--project"])
        .arg(&work)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No path found"));

    // Following edges backwards finds it.
    let out = Command::new(BIN)
        .args([
            "path",
            "decorate",
            "main",
            "--direction",
            "reverse",
            "--json",
            "--project",
        ])
        .arg(&work)
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        support::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed[0]["found"], true);

    // An unknown symbol is an error.
    let out = Command::new(BIN)
        .args(["path", "no_such_symbol_xyz", "main", "--project"])
        .arg(&work)
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn explain_summarizes_symbol() {
    let work = workdir("rust_app", "explain_cmd");

    let out = Command::new(BIN)
        .args(["explain", "greet", "--json", "--project"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl explain");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value =
        support::from_str(&String::from_utf8_lossy(&out.stdout)).expect("valid JSON");
    let report = &parsed.as_array().expect("array")[0];
    assert_eq!(report["symbol"], "greet");
    assert_eq!(report["is_exported"], true);
    assert!(report["signature"].as_str().unwrap().contains("fn greet"));
    assert_eq!(report["relations"]["callers"][0]["label"], "main");
    assert_eq!(report["relations"]["callees"][0]["label"], "decorate");

    let out = Command::new(BIN)
        .args(["explain", "decorate", "--project"])
        .arg(&work)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("EXPLAIN: decorate"), "stdout: {stdout}");
    assert!(
        stdout.contains("Visibility     : private"),
        "stdout: {stdout}"
    );
}

#[test]
fn impact_crosses_module_follows_communities() {
    let work = workdir("two_modules", "impact_communities");

    let run = |symbol: &str| -> serde_json::Value {
        let out = Command::new(BIN)
            .args(["impact", symbol, "--json", "--project"])
            .arg(&work)
            .output()
            .expect("failed to run code-rcl impact");
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        support::from_str::<serde_json::Value>(&String::from_utf8_lossy(&out.stdout))
            .expect("valid JSON")[0]
            .clone()
    };

    // `tax::apply` is only used by callers in its own cluster.
    let apply = run("apply");
    let own_cluster = apply["target_community"]
        .as_str()
        .expect("target community")
        .to_string();
    for caller in apply["callers"].as_array().unwrap() {
        assert_eq!(caller["crosses_module"], false, "{caller}");
        assert_eq!(caller["community"], own_cluster.as_str());
    }

    // `parcel::ship` is called from the other cluster and calls into its own.
    let ship = run("ship");
    let ship_cluster = ship["target_community"].as_str().expect("target community");
    assert_ne!(ship_cluster, own_cluster);

    let caller = &ship["callers"][0];
    assert_eq!(caller["label"], "send_receipt");
    assert_eq!(caller["crosses_module"], true, "{caller}");
    assert_eq!(caller["community"], own_cluster.as_str());
    assert!(ship["risky_affected_count"].as_u64().unwrap() >= 1);

    for callee in ship["callees"].as_array().unwrap() {
        assert_eq!(callee["crosses_module"], false, "{callee}");
    }

    // The ASCII view names both the target's community and the crossing.
    let out = Command::new(BIN)
        .args(["impact", "ship", "--project"])
        .arg(&work)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Community      :"), "stdout: {stdout}");
    assert!(stdout.contains("⚠ cross-module ("), "stdout: {stdout}");
}

mod support;

#[test]
fn deleted_file_with_unicode_and_spaces_keeps_baseline_callers() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path();
    std::fs::write(work.join("defs.rs"), "pub fn removed() {}\n").unwrap();
    std::fs::write(
        work.join("main.rs"),
        "mod defs; fn main() { defs::removed(); }\n",
    )
    .unwrap();
    std::fs::write(work.join("space ü.rs"), "pub fn extra() {}\n").unwrap();
    assert!(init_git_repo(work));
    std::fs::remove_file(work.join("defs.rs")).unwrap();
    std::fs::remove_file(work.join("space ü.rs")).unwrap();
    let output = Command::new(BIN)
        .args(["impact", "--diff", "--json", "--project"])
        .arg(work)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: serde_json::Value = support::from_slice(&output.stdout).unwrap();
    let deleted = reports
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["target_symbol"] == "removed")
        .expect("deleted definition must be reported");
    assert_eq!(deleted["change_kind"], "deleted");
    assert!(
        deleted["callers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"] == "main")
    );
}

#[test]
fn staged_rename_and_unstaged_edit_are_both_compared_against_head() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("defs.rs"), "pub fn target() {}\n").unwrap();
    std::fs::write(
        root.join("caller ü.rs"),
        "mod defs; pub fn caller() { defs::target(); }\n",
    )
    .unwrap();
    assert!(init_git_repo(root));
    std::fs::rename(root.join("caller ü.rs"), root.join("renamed ü.rs")).unwrap();
    assert!(
        Command::new("git")
            .current_dir(root)
            .args(["add", "-A"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(
        root.join("defs.rs"),
        "pub fn target() { let _changed = 1; }\n",
    )
    .unwrap();
    let output = Command::new(BIN)
        .args(["impact", "--diff", "--json", "--project"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: serde_json::Value = support::from_slice(&output.stdout).unwrap();
    let reports = reports.as_array().unwrap();
    assert!(
        reports
            .iter()
            .any(|r| r["target_path"] == "caller ü.rs" && r["change_kind"] == "deleted")
    );
    assert!(
        reports
            .iter()
            .any(|r| r["target_path"] == "renamed ü.rs" && r["change_kind"] == "added")
    );
    assert!(
        reports
            .iter()
            .any(|r| r["target_symbol"] == "target" && r["change_kind"] == "modified")
    );
}
