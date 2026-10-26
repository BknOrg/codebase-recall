use serde_json::Value;
use std::{fs, process::Command};
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");
#[test]
fn lexical_needs_no_model_and_status_is_explicitly_unverified() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("lib.rs"), "pub fn alpha() {}\n").unwrap();
    let out = Command::new(BIN)
        .args(["search", "alpha", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["exact_symbols"][0]["name"], "alpha");
    assert!(v["data"].get("matches").is_none());
    assert_eq!(v["metadata"]["embeddings"]["configured"], false);
    let out = Command::new(BIN)
        .args(["embeddings", "status", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["metadata"]["freshness"], "unverified");
    assert_eq!(v["data"]["pending_files"], 1);
}
#[test]
fn retrieval_flags_and_missing_model_fail_without_fallback() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".code-rcl")).unwrap();
    fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[embeddings]\nmodel_dir='missing-model'\n",
    )
    .unwrap();
    fs::write(d.path().join("lib.rs"), "fn alpha() {}\n").unwrap();
    for extra in [
        vec!["--semantic", "--hybrid"],
        vec!["--semantic", "--strings"],
        vec!["--hybrid", "--no-grep"],
        vec!["--semantic", "--limit", "101"],
        vec!["--semantic"],
        vec!["--hybrid"],
    ] {
        let out = Command::new(BIN)
            .args(["search", "alpha"])
            .args(extra)
            .arg("--project")
            .arg(d.path())
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            out.stdout.is_empty(),
            "failed retrieval must not print lexical success"
        );
    }
}
#[test]
fn invalid_embedding_configuration_is_rejected() {
    for config in [
        "dimensions=42",
        "batch_size=0",
        "threads=65",
        "model='other'",
        "unknown=true",
        "batching='auto'",
        "chunk_profile='unknown'",
    ] {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".code-rcl")).unwrap();
        fs::write(
            d.path().join(".code-rcl/config.toml"),
            format!("[embeddings]\n{config}\n"),
        )
        .unwrap();
        let out = Command::new(BIN)
            .args(["embeddings", "status", "--project"])
            .arg(d.path())
            .output()
            .unwrap();
        assert!(!out.status.success(), "{config}");
    }
}
#[cfg(feature = "embeddings")]
#[test]
#[ignore = "requires CODE_RCL_EMBEDDING_MODEL_DIR; real-model CI job only"]
fn real_model_setup_index_search_and_partial_update() {
    let model = std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR").unwrap();
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("cancel.rs"),"pub fn cancel_request(token: &std::sync::atomic::AtomicBool) { token.store(true, std::sync::atomic::Ordering::Relaxed); }\n").unwrap();
    fs::write(
        d.path().join("sort.rs"),
        "pub fn sort_names(names: &mut [String]) { names.sort(); }\n",
    )
    .unwrap();
    fs::write(d.path().join("unicode.rs"), format!("/// Unicode source chunking 日本語🙂\npub fn unicode_example() {{ let text = \"{}\"; }}\n", "日本語🙂".repeat(300))).unwrap();
    let setup = Command::new(BIN)
        .args(["embeddings", "setup", "--model-dir", &model, "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let sync = Command::new(BIN)
        .args(["embeddings", "sync", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
    // Changing only the CPU budget must preserve valid vectors and avoid inference.
    let config_path = d.path().join(".code-rcl/config.toml");
    let original_config = fs::read_to_string(&config_path).unwrap();
    let mut config: toml::Value = toml::from_str(&original_config).unwrap();
    config["embeddings"]
        .as_table_mut()
        .unwrap()
        .insert("threads".into(), toml::Value::Integer(4));
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let unchanged = Command::new(BIN)
        .args([
            "embeddings",
            "sync",
            "--json",
            "--progress",
            "off",
            "--project",
        ])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        unchanged.status.success(),
        "{}",
        String::from_utf8_lossy(&unchanged.stderr)
    );
    let unchanged: Value = serde_json::from_slice(&unchanged.stdout).unwrap();
    assert_eq!(unchanged["data"]["embedded_chunks"], 0);
    assert!(
        unchanged["metadata"]["timings_ms"]
            .get("model_load")
            .is_none(),
        "no-op sync must not initialize inference"
    );
    fs::write(&config_path, original_config).unwrap();
    let automatic = Command::new(BIN)
        .args([
            "embeddings",
            "sync",
            "--json",
            "--progress",
            "off",
            "--project",
        ])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        automatic.status.success(),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    let automatic: Value = serde_json::from_slice(&automatic.stdout).unwrap();
    assert_eq!(automatic["data"]["embedded_chunks"], 0);
    assert!(
        automatic["metadata"]["timings_ms"]
            .get("model_load")
            .is_none()
    );
    for flag in ["--semantic", "--hybrid"] {
        let out = Command::new(BIN)
            .args(["search", "cancel request", flag, "--json", "--project"])
            .arg(d.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["data"]["matches"][0]["name"], "cancel_request");
        assert_eq!(v["status"], "ok");
    }
    fs::write(d.path().join("cancel.rs"), "pub fn stopped() {}\n").unwrap();
    let source_sync = Command::new(BIN)
        .args(["sync", "--no-report", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        source_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&source_sync.stderr)
    );
    let out = Command::new(BIN)
        .args([
            "search",
            "cancel request",
            "--semantic",
            "--no-sync",
            "--json",
            "--project",
        ])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "partial");
    assert_ne!(v["metadata"]["resolution_mode"], "mixed");
    assert!(
        v["data"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["path"] != "cancel.rs")
    );
    let fresh = Command::new(BIN)
        .args(["search", "stopped", "--semantic", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    let fresh: Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_eq!(fresh["status"], "ok");
    assert_eq!(fresh["metadata"]["embeddings"]["pending_files"], 0);
    assert!(
        fresh["metadata"]["retrieval"]["index_refresh"]["embedded_chunks"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        fresh["data"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["name"] == "stopped")
    );
}

#[test]
fn status_uses_operation_snapshot_while_database_is_locked() {
    use serde_json::json;
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".code-rcl")).unwrap();
    let _db = bkndb::BknDb::open(d.path().join(".code-rcl/cache.bkndb")).unwrap();
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(d.path().join(".code-rcl/embedding-operation.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    fs::write(
        d.path().join(".code-rcl/embedding-operation.json"),
        json!({"state":"running","phase":"inference","last_index":{"indexed_chunks":3}})
            .to_string(),
    )
    .unwrap();
    let out = Command::new(BIN)
        .args(["embeddings", "status", "--json", "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["operation"]["active"], true);
    assert_eq!(v["data"]["indexed_chunks"], 3);
    assert_eq!(v["data"]["freshness"], "unverified");
    assert_eq!(v["data"]["database_busy"], true);
}
#[test]
fn failed_sync_has_immediate_stderr_progress_and_one_json_result() {
    for mode in ["plain", "off"] {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".code-rcl")).unwrap();
        fs::write(
            d.path().join(".code-rcl/config.toml"),
            "[embeddings]\nmodel_dir='missing-model'\n",
        )
        .unwrap();
        let out = Command::new(BIN)
            .args([
                "embeddings",
                "sync",
                "--json",
                "--progress",
                mode,
                "--project",
            ])
            .arg(d.path())
            .output()
            .unwrap();
        assert!(!out.status.success());
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["status"], "error");
        assert_eq!(v["metadata"]["operation"]["state"], "failed");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr.contains("embedding running: opening_cache"),
            mode == "plain"
        );
    }
}
#[test]
fn operation_ownership_failure_still_returns_one_json_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".code-rcl")).unwrap();
    let lock = fs::File::create(dir.path().join(".code-rcl/embedding-operation.lock")).unwrap();
    lock.lock().unwrap();
    let out = Command::new(BIN)
        .args([
            "embeddings",
            "sync",
            "--json",
            "--progress",
            "off",
            "--project",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["status"], "error");
    assert!(
        value["metadata"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("another embedding operation")
    );
}
#[test]
fn staging_symlink_or_junction_cannot_escape_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let foreign = dir.path().join("foreign");
    fs::create_dir_all(project.join(".code-rcl")).unwrap();
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep"), "untouched").unwrap();
    let link = project.join(".code-rcl/embedding-staging");
    #[cfg(windows)]
    {
        let out = Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(foreign.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&foreign, &link).unwrap();
    let out = Command::new(BIN)
        .args([
            "embeddings",
            "sync",
            "--json",
            "--progress",
            "off",
            "--project",
        ])
        .arg(&project)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["status"], "error");
    assert!(
        value["metadata"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("staging")
    );
    assert_eq!(
        fs::read_to_string(foreign.join("keep")).unwrap(),
        "untouched"
    );
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
}
