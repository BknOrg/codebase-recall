use anyhow::{Result, bail};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::SetupArgs;
use crate::commands::setup_integrations as integ;

/// Master AI Agent Skill embedded directly into the binary at compile time.
const REFERENCES: &[(&str, &str)] = &[
    (
        "precise-selective.md",
        include_str!("../../skills/code-rcl/references/precise-selective.md"),
    ),
    (
        "c-family.md",
        include_str!("../../skills/code-rcl/references/c-family.md"),
    ),
    (
        "symbols.md",
        include_str!("../../skills/code-rcl/references/symbols.md"),
    ),
    (
        "embeddings.md",
        include_str!("../../skills/code-rcl/references/embeddings.md"),
    ),
    (
        "cli.md",
        include_str!("../../skills/code-rcl/references/cli.md"),
    ),
    (
        "mcp.md",
        include_str!("../../skills/code-rcl/references/mcp.md"),
    ),
    (
        "installation.md",
        include_str!("../../skills/code-rcl/references/installation.md"),
    ),
];

const EMBEDDED_SKILL: &str = include_str!("../../skills/code-rcl/SKILL.md");

/// Bundle documentation describes the executable shipping it, not a future release.
pub(crate) fn bundled_text(text: &str) -> String {
    let version = EMBEDDED_SKILL
        .lines()
        .find_map(|line| line.trim().strip_prefix("version: "))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .trim_matches('"');
    text.replace(version, env!("CARGO_PKG_VERSION"))
}

/// Cleans Windows verbatim path prefix (\\?\) if present.
fn clean_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s
    }
}

/// Detects home directory across Windows and Unix platforms.
fn get_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE")
            && !profile.is_empty()
        {
            return Some(PathBuf::from(profile));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// Name the installer puts on `PATH`, and what config files should invoke.
const COMMAND_NAME: &str = "code-rcl";

/// Resolves the command string written into MCP configs and hooks.
///
/// The bare `code-rcl` when it is on `PATH`: config files get committed and
/// shared across machines and platforms, and an absolute path such as
/// `C:\Users\me\.code-rcl\bin\code-rcl.exe` is valid on exactly one of them.
/// Only when it is not on `PATH` does this fall back to the absolute path of the
/// running executable, so a fresh setup still works.
fn get_executable_command() -> String {
    let exe = std::env::current_exe()
        .ok()
        .map(|exe| clean_path(&exe.canonicalize().unwrap_or(exe)));
    command_for(exe, std::env::var_os("PATH").as_deref())
}

fn command_for(exe: Option<String>, path_var: Option<&std::ffi::OsStr>) -> String {
    if on_path(COMMAND_NAME, path_var) {
        return COMMAND_NAME.to_string();
    }
    exe.unwrap_or_else(|| COMMAND_NAME.to_string())
}

/// Whether an executable called `name` sits in a directory of `path_var`.
fn on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> bool {
    let Some(path_var) = path_var else {
        return false;
    };
    let candidates: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".com"]
    } else {
        &[""]
    };
    std::env::split_paths(path_var).any(|dir| {
        candidates
            .iter()
            .any(|ext| dir.join(format!("{name}{ext}")).is_file())
    })
}

#[path = "agent_setup.rs"]
mod agents;

pub fn run(args: SetupArgs) -> Result<()> {
    if args.print_skill {
        println!("{}", bundled_text(EMBEDDED_SKILL));
        return Ok(());
    }
    if args.print_reminder {
        println!("{}", integ::REMINDER_TEXT);
        return Ok(());
    }
    if args.remove {
        return apply_integrations(
            &args,
            &get_executable_command(),
            get_home_dir().as_deref(),
            true,
        );
    }
    agents::run(&args, false)
}

pub fn uninstall(args: crate::cli::UninstallArgs) -> Result<()> {
    agents::run(
        &SetupArgs {
            global: args.global,
            workspace: args.workspace,
            target: args.target,
            skill_only: args.skill_only,
            mcp_only: args.mcp_only,
            skill_dir: args.skill_dir,
            print_skill: false,
            print_reminder: false,
            remove: false,
            instructions: false,
            git_hook: false,
            claude_hook: false,
        },
        true,
    )
}

fn report(what: &str, path: &Path, change: integ::Change) {
    println!("  ✓ {what} {}: {}", change.describe(), path.display());
}

/// Install (or, with `remove`, uninstall) the opt-in integrations. With `remove` and no
/// integration flag given, all three are removed.
fn apply_integrations(
    args: &SetupArgs,
    exe_cmd: &str,
    home: Option<&Path>,
    remove: bool,
) -> Result<()> {
    let none_selected = !(args.instructions || args.git_hook || args.claude_hook);
    let do_instructions = args.instructions || (remove && none_selected);
    let do_git_hook = args.git_hook || (remove && none_selected);
    let do_claude_hook = args.claude_hook || (remove && none_selected);

    let base: PathBuf = if args.global {
        match home {
            Some(h) => h.to_path_buf(),
            None => bail!("cannot determine the home directory for --global integrations"),
        }
    } else {
        PathBuf::from(".")
    };

    if do_instructions {
        for file in integ::instruction_files(&base, args.global, &args.target) {
            let change = if remove {
                integ::remove_marked_block(&file, integ::MD_BEGIN, integ::MD_END)?
            } else {
                integ::upsert_marked_block(
                    &file,
                    integ::MD_BEGIN,
                    integ::MD_END,
                    integ::INSTRUCTIONS_BODY,
                )?
            };
            report("Agent instructions", &file, change);
        }
    }

    if do_git_hook {
        if args.global {
            eprintln!(
                "  ⚠ Skipped git hook: hooks are per-repository; run without --global inside the repo"
            );
        } else {
            let hook = integ::post_commit_hook_path(&base)?;
            let change = if remove {
                integ::remove_git_hook(&hook)?
            } else {
                integ::install_git_hook(&hook, exe_cmd)?
            };
            report("Git post-commit hook", &hook, change);
        }
    }

    if do_claude_hook {
        let target = args.target.to_lowercase();
        if target == "all" || target == "claude" {
            let settings = base.join(".claude").join("settings.json");
            let change = if remove {
                integ::remove_claude_hook(&settings)?
            } else {
                integ::install_claude_hook(&settings, exe_cmd)?
            };
            report("Claude Code hook", &settings, change);
        } else {
            eprintln!(
                "  ⚠ Skipped Claude Code hook: --target {} is not claude/all",
                args.target
            );
        }
    }

    if remove {
        println!("[code-rcl setup] Done.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_cli_examples_parse_with_the_current_binary_contract() {
        use clap::Parser;
        let reference = REFERENCES
            .iter()
            .find(|(name, _)| *name == "cli.md")
            .unwrap()
            .1;
        let examples = reference
            .lines()
            .filter(|line| line.starts_with("code-rcl "))
            .collect::<Vec<_>>();
        assert!(!examples.is_empty());
        for example in examples {
            crate::cli::Cli::try_parse_from(example.split_whitespace())
                .unwrap_or_else(|error| panic!("invalid bundled example {example}: {error}"));
        }
    }

    /// Scoped temp dir; the crate has no dev-dependency for one.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("code-rcl-setup-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn exe_name() -> &'static str {
        if cfg!(windows) {
            "code-rcl.exe"
        } else {
            "code-rcl"
        }
    }

    #[test]
    fn bare_command_when_on_path() {
        let dir = TempDir::new("onpath");
        fs::write(dir.0.join(exe_name()), "").unwrap();
        let path = std::env::join_paths([&dir.0]).unwrap();
        let cmd = command_for(Some("/abs/somewhere/code-rcl".to_string()), Some(&path));
        assert_eq!(cmd, "code-rcl");
    }

    #[test]
    fn absolute_path_only_when_not_on_path() {
        let dir = TempDir::new("offpath");
        fs::write(dir.0.join("something-else"), "").unwrap();
        let path = std::env::join_paths([&dir.0]).unwrap();
        let cmd = command_for(Some("/abs/somewhere/code-rcl".to_string()), Some(&path));
        assert_eq!(cmd, "/abs/somewhere/code-rcl");
        assert_eq!(command_for(None, None), "code-rcl");
    }
}
