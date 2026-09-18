use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::cli::SetupArgs;
use crate::commands::setup_integrations as integ;

/// Master AI Agent Skill embedded directly into the binary at compile time.
const EMBEDDED_SKILL: &str = include_str!("../../skills/code-rcl/SKILL.md");

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
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
    }
    std::env::var("HOME").ok().map(PathBuf::from)
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

/// Inserts or updates the "code-rcl" entry in an MCP configuration JSON file.
fn update_mcp_json_file(file_path: &Path, exe_cmd: &str) -> Result<()> {
    let mut root: Value = if file_path.exists() {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read MCP config: {}", file_path.display()))?;
        if content.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&content).with_context(|| {
                format!(
                    "{} is not valid JSON; refusing to overwrite it. Fix the file, then re-run",
                    file_path.display()
                )
            })?
        }
    } else {
        json!({})
    };

    if !root.is_object() {
        bail!(
            "{} must contain a JSON object; refusing to overwrite it",
            file_path.display()
        );
    }

    let mcp_servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    if !mcp_servers.is_object() {
        bail!(
            "{}: \"mcpServers\" must be an object; refusing to overwrite it",
            file_path.display()
        );
    }

    mcp_servers.as_object_mut().unwrap().insert(
        "code-rcl".to_string(),
        json!({
            "command": exe_cmd,
            "args": ["mcp"]
        }),
    );

    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let pretty_json = serde_json::to_string_pretty(&root)
        .context("Failed to serialize MCP configuration JSON")?;
    fs::write(file_path, format!("{}\n", pretty_json))
        .with_context(|| format!("Failed to write MCP config: {}", file_path.display()))?;

    Ok(())
}

pub fn run(args: SetupArgs) -> Result<()> {
    if args.print_skill {
        println!("{}", EMBEDDED_SKILL);
        return Ok(());
    }
    if args.print_reminder {
        println!("{}", integ::REMINDER_TEXT);
        return Ok(());
    }

    let exe_cmd = get_executable_command();
    let home = get_home_dir();

    if args.remove {
        println!("[code-rcl setup] Removing agent integrations...");
        apply_integrations(&args, &exe_cmd, home.as_deref(), true)?;
        return Ok(());
    }

    println!("[code-rcl setup] Setting up AI agent integration...");

    // 1. Skill Installation
    if !args.mcp_only {
        let mut skill_paths: Vec<(PathBuf, bool)> = Vec::new(); // (path, is_required)

        if let Some(custom_dir) = &args.skill_dir {
            skill_paths.push((custom_dir.join("SKILL.md"), true));
        } else if args.global {
            if let Some(h) = &home {
                skill_paths.push((
                    h.join(".gemini").join("config").join("skills").join("code-rcl").join("SKILL.md"),
                    true,
                ));
            } else {
                eprintln!("[warn] Cannot determine home directory for global skill installation; falling back to workspace.");
                skill_paths.push((
                    PathBuf::from(".agents").join("skills").join("code-rcl").join("SKILL.md"),
                    true,
                ));
            }
        } else if args.workspace {
            skill_paths.push((
                PathBuf::from(".agents").join("skills").join("code-rcl").join("SKILL.md"),
                true,
            ));
        } else {
            // Default: workspace is required, global is best-effort if directory already exists
            skill_paths.push((
                PathBuf::from(".agents").join("skills").join("code-rcl").join("SKILL.md"),
                true,
            ));
            if let Some(h) = &home {
                let global_skill = h.join(".gemini").join("config").join("skills").join("code-rcl").join("SKILL.md");
                if global_skill.parent().map(|p| p.exists()).unwrap_or(false) {
                    skill_paths.push((global_skill, false));
                }
            }
        }

        for (skill_path, required) in skill_paths {
            if let Some(parent) = skill_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    if required {
                        return Err(e).with_context(|| format!("Failed to create skill directory: {}", parent.display()));
                    } else {
                        eprintln!("  ⚠ Skipped optional skill directory: {}", parent.display());
                        continue;
                    }
                }
            }
            match fs::write(&skill_path, EMBEDDED_SKILL) {
                Ok(_) => println!("  ✓ Installed agent skill: {}", skill_path.display()),
                Err(e) => {
                    if required {
                        return Err(e).with_context(|| format!("Failed to write SKILL.md to: {}", skill_path.display()));
                    } else {
                        eprintln!("  ⚠ Skipped optional skill path: {}", skill_path.display());
                    }
                }
            }
        }
    }

    // 2. MCP Server Configuration
    if !args.skill_only {
        let target_lower = args.target.to_lowercase();
        let config_gemini = target_lower == "all" || target_lower == "gemini" || target_lower == "antigravity";
        let config_claude = target_lower == "all" || target_lower == "claude";

        if args.global {
            if config_gemini {
                if let Some(h) = &home {
                    let global_gemini = h.join(".gemini").join("config").join("mcp_config.json");
                    update_mcp_json_file(&global_gemini, &exe_cmd)?;
                    println!("  ✓ Registered global MCP server (Antigravity/Gemini): {}", global_gemini.display());
                }
            }
            if config_claude {
                if let Some(h) = &home {
                    let global_claude = h.join(".claude.json");
                    update_mcp_json_file(&global_claude, &exe_cmd)?;
                    println!("  ✓ Registered global MCP server (Claude): {}", global_claude.display());
                }
            }
        } else if args.workspace {
            // Explicit workspace installation only
            if config_claude {
                let workspace_mcp = PathBuf::from(".mcp.json");
                update_mcp_json_file(&workspace_mcp, &exe_cmd)?;
                println!("  ✓ Registered workspace MCP server (Claude Code / Standard): {}", workspace_mcp.display());
            }

            if config_gemini {
                let workspace_gemini = PathBuf::from(".agents").join("mcp_config.json");
                update_mcp_json_file(&workspace_gemini, &exe_cmd)?;
                println!("  ✓ Registered workspace MCP server (Antigravity/Gemini): {}", workspace_gemini.display());
            }
        } else {
            // Default: workspace configs are primary, global Gemini is synchronized if available
            if config_claude {
                let workspace_mcp = PathBuf::from(".mcp.json");
                update_mcp_json_file(&workspace_mcp, &exe_cmd)?;
                println!("  ✓ Registered workspace MCP server (Claude Code / Standard): {}", workspace_mcp.display());
            }

            if config_gemini {
                let workspace_gemini = PathBuf::from(".agents").join("mcp_config.json");
                update_mcp_json_file(&workspace_gemini, &exe_cmd)?;
                println!("  ✓ Registered workspace MCP server (Antigravity/Gemini): {}", workspace_gemini.display());

                // Best-effort sync to global config if directory exists
                if let Some(h) = &home {
                    let global_gemini = h.join(".gemini").join("config").join("mcp_config.json");
                    if global_gemini.parent().map(|p| p.exists()).unwrap_or(false) {
                        if let Err(e) = update_mcp_json_file(&global_gemini, &exe_cmd) {
                            eprintln!("  ⚠ Optional global MCP config sync skipped: {}", e);
                        } else {
                            println!("  ✓ Synchronized global MCP server (Antigravity/Gemini): {}", global_gemini.display());
                        }
                    }
                }
            }
        }
    }

    if args.instructions || args.git_hook || args.claude_hook {
        apply_integrations(&args, &exe_cmd, home.as_deref(), false)?;
    }

    println!("[code-rcl setup] Setup completed successfully.");
    println!("  Command written to configs: {}", exe_cmd);
    if exe_cmd != COMMAND_NAME {
        println!(
            "  ⚠ `{COMMAND_NAME}` is not on PATH, so the absolute path was written; \
             add its folder to PATH and re-run setup to get a portable config."
        );
    }
    println!("  Run `code-rcl --help` or `code-rcl mcp` for details.");

    Ok(())
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
            eprintln!("  ⚠ Skipped git hook: hooks are per-repository; run without --global inside the repo");
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
            eprintln!("  ⚠ Skipped Claude Code hook: --target {} is not claude/all", args.target);
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

    /// Scoped temp dir; the crate has no dev-dependency for one.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("code-rcl-setup-{}-{tag}", std::process::id()));
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
        if cfg!(windows) { "code-rcl.exe" } else { "code-rcl" }
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
