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
fn digest_help_shows_parameters_and_examples() {
    let out = Command::new(BIN)
        .args(["digest", "help"])
        .output()
        .expect("failed to run code-rcl digest help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("code-rcl digest"));
}

#[test]
fn digest_generates_markdown_outline() {
    let work = workdir("rust_app", "digest_md");

    let out = Command::new(BIN)
        .arg("digest")
        .arg(&work)
        .output()
        .expect("failed to run code-rcl digest");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Architecture Digest:"));
    assert!(stdout.contains("Modules & Public APIs"));
    assert!(stdout.contains("util.rs"));
    assert!(stdout.contains("pub fn greet"));
}

#[test]
fn digest_json_format_is_valid() {
    let work = workdir("rust_app", "digest_json");

    let out = Command::new(BIN)
        .args(["digest", "--json"])
        .arg(&work)
        .output()
        .expect("failed to run code-rcl digest --json");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let val: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid json from digest --json");
    assert!(val.get("project").is_some());
    assert!(val.get("total_files").is_some());
    assert!(val.get("modules").is_some());
}

#[test]
fn digest_output_file_flag_works() {
    let work = workdir("rust_app", "digest_file");
    let out_file = work.join("my-arch.md");

    let out = Command::new(BIN)
        .args(["digest", "-o"])
        .arg(&out_file)
        .arg(&work)
        .output()
        .expect("failed to run code-rcl digest -o");
    assert!(out.status.success());
    assert!(out_file.exists(), "my-arch.md was not written");
    let content = std::fs::read_to_string(&out_file).unwrap();
    assert!(content.contains("# Architecture Digest:"));
    assert!(content.contains("pub fn greet"));
}
