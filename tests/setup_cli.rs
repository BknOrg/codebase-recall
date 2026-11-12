use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

/// A scratch git repo; HOME/USERPROFILE point inside it so setup can never touch the real home.
fn scratch_repo(tag: &str) -> Option<PathBuf> {
    let out = Command::new("git").arg("--version").output();
    if out.is_err() || !out.as_ref().unwrap().status.success() {
        eprintln!("skipping test: git must be installed to run this test");
        return None;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("setup__{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new("git")
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("git init failed");
    assert!(out.status.success());
    Some(dir)
}

fn setup(dir: &Path, args: &[&str]) -> Output {
    integration(dir, "setup", args)
}

fn integration(dir: &Path, command: &str, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg(command)
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("CODEX_HOME", dir.join(".codex"))
        .output()
        .expect("failed to run code-rcl setup")
}

fn success(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn native_agent_targets_roundtrip_workspace_and_global() {
    for scope in ["--workspace", "--global"] {
        for target in ["codex", "claude", "antigravity", "gemini", "all"] {
            let d = tempfile::tempdir().unwrap();
            let dir = d.path();
            let args = [scope, "--target", target];
            success(setup(dir, &args));
            assert!(success(setup(dir, &args)).contains("already up to date"));
            let global = scope == "--global";
            let codex = target == "all" || target == "codex";
            let claude = target == "all" || target == "claude";
            let gemini = matches!(target, "all" | "antigravity" | "gemini");
            if codex {
                let text = read(dir, ".codex/config.toml");
                let doc: toml::Value = toml::from_str(&text).unwrap();
                let args = doc["mcp_servers"]["code-rcl"]["args"].as_array().unwrap();
                assert_eq!(args.len(), if global { 1 } else { 3 });
                assert!(dir.join(".agents/skills/code-rcl/SKILL.md").exists());
            }
            if claude {
                assert!(dir.join(".claude/skills/code-rcl/SKILL.md").exists());
                assert!(
                    dir.join(if global { ".claude.json" } else { ".mcp.json" })
                        .exists()
                );
            }
            if gemini {
                assert!(
                    dir.join(if global {
                        ".gemini/config/skills/code-rcl/SKILL.md"
                    } else {
                        ".agents/skills/code-rcl/SKILL.md"
                    })
                    .exists()
                );
            }
            success(integration(dir, "uninstall", &args));
            success(integration(dir, "uninstall", &args));
            for p in [
                ".agents/skills/code-rcl/SKILL.md",
                ".claude/skills/code-rcl/SKILL.md",
                ".gemini/config/skills/code-rcl/SKILL.md",
            ] {
                assert!(!dir.join(p).exists(), "{scope}/{target}: {p}");
            }
            assert!(!dir.join(".code-rcl/agent-integrations.json").exists());
        }
    }
}

#[test]
fn uninstall_retains_shared_skill_instructions_and_extra_files() {
    let Some(dir) = scratch_repo("shared_uninstall") else {
        return;
    };
    success(setup(
        &dir,
        &["--instructions", "--git-hook", "--claude-hook"],
    ));
    std::fs::write(dir.join(".agents/skills/code-rcl/notes.txt"), "mine").unwrap();
    std::fs::write(
        dir.join(".agents/skills/code-rcl/references/custom.md"),
        "mine",
    )
    .unwrap();
    success(integration(&dir, "uninstall", &["--target", "codex"]));
    assert!(dir.join(".agents/skills/code-rcl/SKILL.md").exists());
    assert!(read(&dir, "AGENTS.md").contains("code-rcl:begin"));
    let codex: toml::Value = toml::from_str(&read(&dir, ".codex/config.toml")).unwrap();
    assert!(codex["mcp_servers"].get("code-rcl").is_none());
    success(integration(&dir, "uninstall", &["--target", "gemini"]));
    assert!(!dir.join(".agents/skills/code-rcl/SKILL.md").exists());
    assert!(!dir.join("AGENTS.md").exists());
    assert_eq!(read(&dir, ".agents/skills/code-rcl/notes.txt"), "mine");
    assert_eq!(
        read(&dir, ".agents/skills/code-rcl/references/custom.md"),
        "mine"
    );
    assert!(dir.join(".git/hooks/post-commit").exists());
    success(integration(&dir, "uninstall", &["--target", "claude"]));
    assert!(!dir.join(".git/hooks/post-commit").exists());
    assert!(!dir.join(".claude/settings.json").exists());
}

#[test]
fn setup_preflight_preserves_toml_comments_json_and_user_settings() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    std::fs::create_dir_all(dir.join(".codex")).unwrap();
    let original =
        "# my config\nmodel = 'keep-me' # keep comment\n[mcp_servers.other]\ncommand = 'other'\n";
    std::fs::write(dir.join(".codex/config.toml"), original).unwrap();
    std::fs::write(dir.join(".mcp.json"), "{ broken").unwrap();
    let out = setup(dir, &[]);
    assert!(!out.status.success());
    assert_eq!(read(dir, ".codex/config.toml"), original);
    assert!(!dir.join(".agents/skills/code-rcl").exists());
    std::fs::write(
        dir.join(".mcp.json"),
        r#"{"otherSetting":true,"mcpServers":{"other":{"command":"other"}}}"#,
    )
    .unwrap();
    success(setup(dir, &[]));
    assert!(read(dir, ".codex/config.toml").contains("# keep comment"));
    success(integration(dir, "uninstall", &[]));
    let text = read(dir, ".codex/config.toml");
    assert!(text.contains("# my config") && text.contains("# keep comment"));
    let doc: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(doc["model"].as_str(), Some("keep-me"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
    let json: serde_json::Value = serde_json::from_str(&read(dir, ".mcp.json")).unwrap();
    assert_eq!(json["otherSetting"], true);
    assert_eq!(json["mcpServers"]["other"]["command"], "other");
}

#[test]
fn uninstall_legacy_shared_bundle_is_conservative_and_all_can_remove_it() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let bundle = dir.join(".agents/skills/code-rcl");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(
        bundle.join("SKILL.md"),
        "---\nname: code-rcl\n---\nold skill\n",
    )
    .unwrap();
    assert!(
        success(integration(dir, "uninstall", &["--target", "codex"]))
            .contains("Retained shared/legacy")
    );
    assert!(bundle.join("SKILL.md").exists());
    success(integration(dir, "uninstall", &["--target", "all"]));
    assert!(!bundle.exists());
}

#[test]
fn component_selectors_and_custom_skill_directory_are_respected() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let args = [
        "--target",
        "codex",
        "--skill-only",
        "--skill-dir",
        "custom skill",
    ];
    success(setup(dir, &args));
    assert!(dir.join("custom skill/SKILL.md").exists());
    assert!(!dir.join(".codex/config.toml").exists());
    success(integration(dir, "uninstall", &args));
    assert!(!dir.join("custom skill").exists());
    success(setup(dir, &["--target", "claude", "--mcp-only"]));
    assert!(!dir.join(".claude/skills").exists());
    success(integration(
        dir,
        "uninstall",
        &["--target", "claude", "--skill-only"],
    ));
    let json: serde_json::Value = serde_json::from_str(&read(dir, ".mcp.json")).unwrap();
    assert!(json["mcpServers"].get("code-rcl").is_some());
}

#[test]
fn invalid_target_and_conflicting_flags_do_not_write_anything() {
    for command in ["setup", "uninstall"] {
        for args in [
            &["--target", "typo"][..],
            &["--workspace", "--global"],
            &["--skill-only", "--mcp-only"],
            &["--target", "codex", "--skill-dir", ".codex/config.toml"],
        ] {
            let d = tempfile::tempdir().unwrap();
            assert!(!integration(d.path(), command, args).status.success());
            assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn codex_global_respects_custom_home_and_inline_tables() {
    let home = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let config = codex.path().join("config.toml");
    std::fs::write(
        &config,
        "# keep\nmodel='my-model'\nmcp_servers={other={command='other'}}\n",
    )
    .unwrap();
    let run = |command| {
        Command::new(BIN)
            .args([command, "--global", "--target", "codex"])
            .current_dir(home.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("CODEX_HOME", codex.path())
            .output()
            .unwrap()
    };
    success(run("setup"));
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("# keep"));
    let doc: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        doc["mcp_servers"]["code-rcl"]["args"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    success(run("uninstall"));
    let doc: toml::Value = toml::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
    assert_eq!(doc["model"].as_str(), Some("my-model"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
    assert!(!home.path().join(".codex").exists());
}

#[test]
fn broken_codex_toml_prevents_every_setup_destination_write() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(".codex")).unwrap();
    std::fs::write(d.path().join(".codex/config.toml"), "[broken").unwrap();
    assert!(!setup(d.path(), &[]).status.success());
    assert_eq!(read(d.path(), ".codex/config.toml"), "[broken");
    assert!(!d.path().join(".agents").exists());
    assert!(!d.path().join(".claude").exists());
    assert!(!d.path().join(".code-rcl").exists());
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"))
}

#[test]
fn setup_integrations_install_are_idempotent_and_removable() {
    let Some(dir) = scratch_repo("roundtrip") else {
        return;
    };
    let flags = [
        "--workspace",
        "--instructions",
        "--git-hook",
        "--claude-hook",
    ];

    let out = setup(&dir, &flags);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let skill = read(&dir, ".agents/skills/code-rcl/SKILL.md");
    assert!(skill.contains(env!("CARGO_PKG_VERSION")));
    assert!(skill.lines().count() <= 150);
    for name in [
        "cli.md",
        "mcp.md",
        "installation.md",
        "embeddings.md",
        "symbols.md",
        "c-family.md",
    ] {
        assert!(
            read(&dir, &format!(".agents/skills/code-rcl/references/{name}"))
                .contains(env!("CARGO_PKG_VERSION"))
        );
    }
    let config: serde_json::Value = serde_json::from_str(&read(&dir, ".mcp.json")).unwrap();
    assert_eq!(config["mcpServers"]["code-rcl"]["args"][1], "--project");
    assert!(
        Path::new(
            config["mcpServers"]["code-rcl"]["args"][2]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );

    for file in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let content = read(&dir, file);
        assert!(
            content.contains("<!-- code-rcl:begin -->"),
            "{file}: {content}"
        );
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
    let snapshot: Vec<String> = [
        "CLAUDE.md",
        "AGENTS.md",
        ".git/hooks/post-commit",
        ".claude/settings.json",
    ]
    .iter()
    .map(|f| read(&dir, f))
    .collect();
    let out = setup(&dir, &flags);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("already up to date"));
    let again: Vec<String> = [
        "CLAUDE.md",
        "AGENTS.md",
        ".git/hooks/post-commit",
        ".claude/settings.json",
    ]
    .iter()
    .map(|f| read(&dir, f))
    .collect();
    assert_eq!(snapshot, again);

    // --remove undoes exactly what we added.
    let out = setup(&dir, &["--workspace", "--remove"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for file in [
        "CLAUDE.md",
        "AGENTS.md",
        "GEMINI.md",
        ".git/hooks/post-commit",
        ".claude/settings.json",
    ] {
        assert!(!dir.join(file).exists(), "{file} should have been removed");
    }
}

#[test]
fn setup_preserves_user_content_and_refuses_broken_settings() {
    let Some(dir) = scratch_repo("preserve") else {
        return;
    };
    std::fs::write(dir.join("CLAUDE.md"), "# Mine\n\nkeep this line\n").unwrap();
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    std::fs::write(dir.join(".claude/settings.json"), "{ not json").unwrap();

    // Broken settings.json: setup fails and leaves the file exactly as it was.
    let out = setup(
        &dir,
        &[
            "--workspace",
            "--skill-only",
            "--instructions",
            "--claude-hook",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not valid JSON"));
    assert_eq!(read(&dir, ".claude/settings.json"), "{ not json");

    // Preflight leaves every destination untouched when any config is malformed.
    let content = read(&dir, "CLAUDE.md");
    assert!(content.starts_with("# Mine\n\nkeep this line\n"));
    assert!(!content.contains("<!-- code-rcl:begin -->"));

    // Removing the block restores the original file byte for byte.
    let out = setup(&dir, &["--workspace", "--remove", "--instructions"]);
    assert!(out.status.success());
    assert_eq!(read(&dir, "CLAUDE.md"), "# Mine\n\nkeep this line\n");
}

#[test]
fn setup_print_reminder_outputs_hook_text() {
    let Some(dir) = scratch_repo("reminder") else {
        return;
    };
    let out = setup(&dir, &["--print-reminder"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("impact"), "stdout: {stdout}");
    // Printing the reminder must not install anything.
    assert!(!dir.join(".mcp.json").exists());
}

#[test]
fn init_appends_to_git_exclude_with_proper_newlines() {
    let Some(dir) = scratch_repo("init_exclude") else {
        return;
    };
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
