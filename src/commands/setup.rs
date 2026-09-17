use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::cli::SetupArgs;

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

/// Resolves the command string to invoke this binary.
/// Prefers the canonical absolute executable path so MCP hosts can find it regardless of PATH.
fn get_executable_command() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            return clean_path(&canonical);
        }
        return clean_path(&exe);
    }
    "code-rcl".to_string()
}

/// Inserts or updates the "code-rcl" entry in an MCP configuration JSON file.
fn update_mcp_json_file(file_path: &Path, exe_cmd: &str) -> Result<()> {
    let mut root: Value = if file_path.exists() {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read MCP config: {}", file_path.display()))?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root.is_object() {
        root = json!({});
    }

    let mcp_servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    if !mcp_servers.is_object() {
        *mcp_servers = json!({});
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

    println!("[code-rcl setup] Setting up AI agent integration...");

    let exe_cmd = get_executable_command();
    let home = get_home_dir();

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

    println!("[code-rcl setup] Setup completed successfully.");
    println!("  Binary path: {}", exe_cmd);
    println!("  Run `code-rcl --help` or `code-rcl mcp` for details.");

    Ok(())
}
