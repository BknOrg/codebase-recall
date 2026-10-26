use serde_json::Value;
use std::process::Command;
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

#[test]
fn fuzzy_filters_apply_before_the_shared_limit() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("lib.rs"),
        "pub fn calculate_total() {}\nfn calculate_totals() {}\npub struct CalculateTotal;\n",
    )
    .unwrap();
    let out = Command::new(BIN)
        .args([
            "search",
            "calclate_total",
            "--lexical",
            "--no-grep",
            "--kind",
            "function",
            "--exported",
            "--limit",
            "1",
            "--json",
            "--project",
        ])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["total_matches"], 1);
    assert_eq!(v["data"]["fuzzy_suggestions"][0]["name"], "calculate_total");
    assert_eq!(v["data"]["match_counts"]["fuzzy_suggestions"], 1);
}

#[test]
fn grep_is_ranked_before_limit_and_no_sync_uses_the_cached_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("a.rs"),
        "// undo unrelated changes\n".repeat(20),
    )
    .unwrap();
    std::fs::write(root.path().join("z.rs"), "// undo transaction\r\n").unwrap();
    let run = |cached: bool| {
        let mut command = Command::new(BIN);
        command
            .args([
                "search",
                "undo transaction",
                "--lexical",
                "--no-fuzzy",
                "--limit",
                "1",
                "--json",
                "--project",
            ])
            .arg(root.path());
        if cached {
            command.arg("--no-sync");
        }
        let out = command.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let v = run(false);
    assert_eq!(v["data"]["grep_matches"][0]["path"], "z.rs");
    assert_eq!(v["data"]["total_matches"], 21);
    assert_eq!(v["data"]["match_counts"]["grep_matches"], 21);
    assert_eq!(v["metadata"]["omitted_count"], 20);
    std::fs::write(root.path().join("z.rs"), "// changed on disk\n").unwrap();
    let cached = run(true);
    assert_eq!(cached["metadata"]["freshness"], "unverified");
    assert_eq!(
        cached["data"]["grep_matches"][0]["snippet"],
        "// undo transaction"
    );
}

#[test]
fn symbol_and_string_results_share_one_budget_and_count_before_truncation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("lib.rs"),
        "pub fn alpha() {}\npub fn alpha_beta() {}\nfn report() { println!(\"alpha\"); }\n",
    )
    .unwrap();
    let out = Command::new(BIN)
        .args([
            "search",
            "alpha",
            "--lexical",
            "--strings",
            "--limit",
            "1",
            "--json",
            "--project",
        ])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["total_matches"], 3);
    assert_eq!(v["data"]["exact_symbols"].as_array().unwrap().len(), 1);
    assert!(v["data"]["string_literals"].as_array().unwrap().is_empty());
    assert_eq!(v["data"]["match_counts"]["string_literals"], 1);
}

#[test]
fn qualified_names_select_the_requested_owner() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("lib.rs"),"mod db { pub struct Transaction; impl Transaction { pub fn commit(&self) {} } }\npub fn commit() {}\n").unwrap();
    let out = Command::new(BIN)
        .args([
            "search",
            "db::Transaction.commit",
            "--lexical",
            "--json",
            "--project",
        ])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["total_matches"], 1);
    assert_eq!(v["data"]["exact_symbols"][0]["kind"], "method");
}

#[test]
fn source_sync_scope_cannot_leak_retained_out_of_scope_symbols() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("lib.rs"), "pub fn shared_name() {}\n").unwrap();
    std::fs::write(
        root.path().join("other.py"),
        "def shared_name():\n    pass\n",
    )
    .unwrap();
    let run = || {
        let out = Command::new(BIN)
            .args(["search", "shared_name", "--lexical", "--json", "--project"])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    assert_eq!(run()["data"]["total_matches"], 2);
    std::fs::write(
        root.path().join(".code-rcl/config.toml"),
        "[sync]\nlanguages=['rust']\n",
    )
    .unwrap();
    let result = run();
    assert_eq!(result["data"]["total_matches"], 1);
    assert_eq!(result["data"]["exact_symbols"][0]["path"], "lib.rs");
}

#[test]
fn indexed_modes_are_case_insensitive_and_distinct_from_substring() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("lib.rs"),
        "pub fn alpha() {}\npub fn alphabet() {}\npub fn pre_alpha() {}\n",
    )
    .unwrap();
    for (mode, expected) in [("exact", 1), ("prefix", 2), ("substring", 3)] {
        let out = Command::new(BIN)
            .args([
                "search",
                "ALPHA",
                "--json",
                "--match-mode",
                mode,
                "--project",
            ])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let result: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            result["data"]["exact_symbols"].as_array().unwrap().len(),
            expected
        );
        assert_eq!(result["metadata"]["freshness"], "verified");
    }
    let out = Command::new(BIN)
        .args([
            "search",
            "alpha",
            "--json",
            "--limit",
            "1",
            "--no-sync",
            "--project",
        ])
        .arg(root.path())
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["metadata"]["freshness"], "unverified");
    assert_eq!(result["metadata"]["truncated"], true);
    assert_eq!(result["data"]["total_matches"], 3);
    assert_eq!(result["metadata"]["omitted_count"], 2);
}

#[test]
fn sync_json_is_a_single_envelope() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("lib.rs"), "fn foo() {}\n").unwrap();
    let output = Command::new(BIN)
        .args(["sync", "--json", "--project"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["data"]["added"], 1);
}
