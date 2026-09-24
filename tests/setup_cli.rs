use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

/// A scratch git repo; HOME/USERPROFILE point inside it so setup can never touch the real home.
fn scratch_repo(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("setup__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new("git")
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("git must be installed to run this test");
    assert!(out.status.success());
    dir
}

fn setup(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("setup")
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .output()
        .expect("failed to run code-rcl setup")
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"))
}

#[test]
fn setup_integrations_install_are_idempotent_and_removable() {
    let dir = scratch_repo("roundtrip");
    let flags = ["--workspace", "--instructions", "--git-hook", "--claude-hook"];

    let out = setup(&dir, &flags);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    for file in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let content = read(&dir, file);
        assert!(content.contains("<!-- code-rcl:begin -->"), "{file}: {content}");
        assert!(content.contains("code-rcl:end"), "{file}");
        assert!(content.contains("impact --diff"), "{file}");
    }
    let hook = read(&dir, ".git/hooks/post-commit");
    assert!(hook.starts_with("#!/bin/sh"));
    assert!(hook.contains("# code-rcl:begin") && hook.contains(" sync "));

    let settings: serde_json::Value =
        serde_json::from_str(&read(&dir, ".claude/settings.json")).expect("valid settings JSON");
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("hook command");
    assert!(command.contains("setup --print-reminder"), "{command}");

    // Second run changes nothing.
    let snapshot: Vec<String> = ["CLAUDE.md", "AGENTS.md", ".git/hooks/post-commit", ".claude/settings.json"]
        .iter()
        .map(|f| read(&dir, f))
        .collect();
    let out = setup(&dir, &flags);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("already up to date"));
    let again: Vec<String> = ["CLAUDE.md", "AGENTS.md", ".git/hooks/post-commit", ".claude/settings.json"]
        .iter()
        .map(|f| read(&dir, f))
        .collect();
    assert_eq!(snapshot, again);

    // --remove undoes exactly what we added.
    let out = setup(&dir, &["--workspace", "--remove"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    for file in ["CLAUDE.md", "AGENTS.md", "GEMINI.md", ".git/hooks/post-commit", ".claude/settings.json"] {
        assert!(!dir.join(file).exists(), "{file} should have been removed");
    }
}

#[test]
fn setup_preserves_user_content_and_refuses_broken_settings() {
    let dir = scratch_repo("preserve");
    std::fs::write(dir.join("CLAUDE.md"), "# Mine\n\nkeep this line\n").unwrap();
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    std::fs::write(dir.join(".claude/settings.json"), "{ not json").unwrap();

    // Broken settings.json: setup fails and leaves the file exactly as it was.
    let out = setup(&dir, &["--workspace", "--skill-only", "--instructions", "--claude-hook"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not valid JSON"));
    assert_eq!(read(&dir, ".claude/settings.json"), "{ not json");

    // The instruction block was added around the user's text, not over it.
    let content = read(&dir, "CLAUDE.md");
    assert!(content.starts_with("# Mine\n\nkeep this line\n"));
    assert!(content.contains("<!-- code-rcl:begin -->"));

    // Removing the block restores the original file byte for byte.
    let out = setup(&dir, &["--workspace", "--remove", "--instructions"]);
    assert!(out.status.success());
    assert_eq!(read(&dir, "CLAUDE.md"), "# Mine\n\nkeep this line\n");
}

#[test]
fn setup_print_reminder_outputs_hook_text() {
    let dir = scratch_repo("reminder");
    let out = setup(&dir, &["--print-reminder"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("impact"), "stdout: {stdout}");
    // Printing the reminder must not install anything.
    assert!(!dir.join(".mcp.json").exists());
}

#[test]
fn init_appends_to_git_exclude_with_proper_newlines() {
    let dir = scratch_repo("init_exclude");
    let exclude = dir.join(".git/info/exclude");

    // Case 1: Existing file without trailing newline
    std::fs::write(&exclude, "some_rule").unwrap();
    let out = Command::new(BIN)
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("failed to run code-rcl init");
    assert!(out.status.success());
    let content = std::fs::read_to_string(&exclude).unwrap();
    assert_eq!(content, "some_rule\n.code-rcl/\n");

    // Case 2: Idempotent - running init again does not re-add
    let out = Command::new(BIN)
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("failed to run code-rcl init");
    assert!(out.status.success());
    let content2 = std::fs::read_to_string(&exclude).unwrap();
    assert_eq!(content2, "some_rule\n.code-rcl/\n");
}

