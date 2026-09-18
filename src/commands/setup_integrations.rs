//! Opt-in "make agents actually use code-rcl" integrations for `setup`:
//! instruction blocks in CLAUDE.md/AGENTS.md/GEMINI.md and a git post-commit hook.
//!
//! Everything here edits files the user owns, so every write is confined to a
//! marker-delimited block that can be updated or removed without touching the
//! surrounding content, and malformed markers abort instead of guessing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const MD_BEGIN: &str = "<!-- code-rcl:begin -->";
pub const MD_END: &str = "<!-- code-rcl:end -->";
pub const SH_BEGIN: &str = "# code-rcl:begin";
pub const SH_END: &str = "# code-rcl:end";

pub const INSTRUCTIONS_BODY: &str = "## code-rcl (codebase graph tools)

This project has `code-rcl` available (MCP tools `code_rcl_*`, or the `code-rcl` CLI).
For structural questions prefer it over grep or reading whole files:

- **Before editing a symbol or file:** run `impact <symbol>` to see its callers and
  callees with risk flags. After editing, run `impact --diff` to see what your
  uncommitted changes affect.
- **Orienting in unfamiliar code:** read `.code-rcl/REPORT.md` (or run `report` /
  `code_rcl_report`) for subsystems, hubs and the seams between them, then `digest` for
  API outlines, instead of opening many files.
- **Understanding one symbol:** run `explain <symbol>` (signature, docs, members,
  direct callers and callees).
- **How does A reach B:** run `path <from> <to>`.
- Use grep only for plain-text searches (comments, strings, config values) that the
  graph does not index.";

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Change {
    Created,
    Updated,
    Unchanged,
    Removed,
    NotPresent,
}

impl Change {
    pub fn describe(self) -> &'static str {
        match self {
            Change::Created => "created",
            Change::Updated => "updated",
            Change::Unchanged => "already up to date",
            Change::Removed => "removed",
            Change::NotPresent => "nothing to remove",
        }
    }
}

fn newline_of(content: &str) -> &'static str {
    if content.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Locate the marker block in `content`. `Ok(None)` when neither marker exists;
/// an error when the markers are unpaired, repeated, or out of order.
fn find_block(path: &Path, content: &str, begin: &str, end: &str) -> Result<Option<(usize, usize)>> {
    let begins = content.matches(begin).count();
    let ends = content.matches(end).count();
    match (begins, ends) {
        (0, 0) => Ok(None),
        (1, 1) => {
            let b = content.find(begin).unwrap();
            let e = content.find(end).unwrap();
            if e < b {
                bail!(
                    "{}: '{end}' appears before '{begin}'; fix or remove the markers by hand",
                    path.display()
                );
            }
            Ok(Some((b, e + end.len())))
        }
        _ => bail!(
            "{}: found {begins} '{begin}' and {ends} '{end}' markers; expected exactly one of each. \
             Fix or remove them by hand, then re-run",
            path.display()
        ),
    }
}

/// Insert or refresh the marker-delimited block in `path`, creating the file if needed.
pub fn upsert_marked_block(path: &Path, begin: &str, end: &str, body: &str) -> Result<Change> {
    let existed = path.exists();
    let content = if existed {
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?
    } else {
        String::new()
    };

    let nl = newline_of(&content);
    let block = format!("{begin}{nl}{}{nl}{end}", body.replace('\n', nl));

    let new_content = match find_block(path, &content, begin, end)? {
        Some((b, e)) => format!("{}{block}{}", &content[..b], &content[e..]),
        None if content.trim().is_empty() => format!("{block}{nl}"),
        None => {
            let mut out = content.clone();
            if !out.ends_with('\n') {
                out.push_str(nl);
            }
            out.push_str(nl);
            out.push_str(&block);
            out.push_str(nl);
            out
        }
    };

    if existed && new_content == content {
        return Ok(Change::Unchanged);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, &new_content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(if existed { Change::Updated } else { Change::Created })
}

/// Remove the marker-delimited block from `path`, deleting the file if nothing else is left.
pub fn remove_marked_block(path: &Path, begin: &str, end: &str) -> Result<Change> {
    if !path.exists() {
        return Ok(Change::NotPresent);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let Some((b, e)) = find_block(path, &content, begin, end)? else {
        return Ok(Change::NotPresent);
    };

    let nl = newline_of(&content);
    let mut before = &content[..b];
    let after = &content[e..];
    let after = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    // Drop the blank separator line we added when inserting the block.
    let double = format!("{nl}{nl}");
    if before.ends_with(&double) {
        before = &before[..before.len() - nl.len()];
    }

    let remaining = format!("{before}{after}");
    if remaining.trim().is_empty() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        fs::write(path, remaining).with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(Change::Removed)
}

/// Instruction files for the selected agent `target` ("all", "claude", "gemini", "antigravity").
pub fn instruction_files(base: &Path, global: bool, target: &str) -> Vec<PathBuf> {
    let target = target.to_lowercase();
    let claude = target == "all" || target == "claude";
    let gemini = target == "all" || target == "gemini" || target == "antigravity";

    let mut files = Vec::new();
    if global {
        if claude {
            files.push(base.join(".claude").join("CLAUDE.md"));
        }
        if gemini {
            files.push(base.join(".gemini").join("GEMINI.md"));
        }
    } else {
        if claude {
            files.push(base.join("CLAUDE.md"));
        }
        if gemini {
            files.push(base.join("GEMINI.md"));
            files.push(base.join("AGENTS.md"));
        }
    }
    files
}

/// Absolute path of the repository's `post-commit` hook, honouring worktrees and `core.hooksPath`.
pub fn post_commit_hook_path(repo_dir: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(repo_dir)
        .output()
        .context("git not found on PATH; the git hook needs git to locate the hooks directory")?;
    if !out.status.success() {
        bail!(
            "'{}' is not inside a git repository; run the git hook setup from a repo",
            repo_dir.display()
        );
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let hooks = PathBuf::from(raw);
    let hooks = if hooks.is_absolute() { hooks } else { repo_dir.join(hooks) };
    Ok(hooks.join("post-commit"))
}

fn shell_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Install (or refresh) the block that keeps the cache fresh after each commit.
pub fn install_git_hook(hook: &Path, exe_cmd: &str) -> Result<Change> {
    let body = format!(
        "# Keep the code-rcl cache fresh in the background; never block or fail the commit.\n(\"{}\" sync >/dev/null 2>&1 &) || true",
        shell_path(exe_cmd)
    );

    let created = !hook.exists();
    if created {
        if let Some(parent) = hook.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(hook, "#!/bin/sh\n")
            .with_context(|| format!("failed to write {}", hook.display()))?;
    } else {
        let existing = fs::read_to_string(hook)
            .with_context(|| format!("failed to read {}", hook.display()))?;
        if let Some(first) = existing.lines().next() {
            if first.starts_with("#!") && (!first.contains("sh") || first.contains("fish")) {
                bail!(
                    "{} uses a non-POSIX-shell interpreter ('{first}'); add `code-rcl sync` to it by hand",
                    hook.display()
                );
            }
        }
    }

    let change = upsert_marked_block(hook, SH_BEGIN, SH_END, &body)?;

    #[cfg(unix)]
    if created {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to make {} executable", hook.display()))?;
    }

    Ok(if created { Change::Created } else { change })
}

/// Remove our block from the hook, deleting the file when only the shebang we added is left.
pub fn remove_git_hook(hook: &Path) -> Result<Change> {
    let change = remove_marked_block(hook, SH_BEGIN, SH_END)?;
    if change == Change::Removed && hook.exists() {
        let left = fs::read_to_string(hook).unwrap_or_default();
        if left.trim() == "#!/bin/sh" {
            fs::remove_file(hook).with_context(|| format!("failed to remove {}", hook.display()))?;
        }
    }
    Ok(change)
}

/// Text printed by `code-rcl setup --print-reminder`; the Claude Code SessionStart
/// hook runs that command and its stdout is added to the session context.
pub const REMINDER_TEXT: &str = "code-rcl is available in this project. For structural questions prefer it over grep: \
`impact <symbol>` before editing (and `impact --diff` after), `.code-rcl/REPORT.md` or `report` \
then `digest` to orient, `explain <symbol>` to understand one symbol, \
`path <from> <to>` to see how two symbols connect.";

/// Substring that identifies the hook handler we own inside settings.json.
const HOOK_MARKER: &str = "setup --print-reminder";

fn read_settings(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "{} is not valid JSON; refusing to overwrite it. Fix the file, then re-run",
            path.display()
        )
    })?;
    if !value.is_object() {
        bail!("{} must contain a JSON object; refusing to overwrite it", path.display());
    }
    Ok(value)
}

fn write_settings(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let pretty = serde_json::to_string_pretty(value).context("failed to serialize settings")?;
    fs::write(path, format!("{pretty}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn is_our_handler(handler: &serde_json::Value) -> bool {
    handler
        .get("command")
        .and_then(|c| c.as_str())
        .is_some_and(|c| c.contains(HOOK_MARKER))
}

/// Add (or refresh) our SessionStart hook in a Claude Code settings.json, leaving every
/// other setting and hook untouched.
pub fn install_claude_hook(path: &Path, exe_cmd: &str) -> Result<Change> {
    use serde_json::{Value, json};

    let existed = path.exists();
    let mut root = read_settings(path)?;
    let original = root.clone();

    let handler = json!({
        "type": "command",
        "command": format!("\"{}\" {HOOK_MARKER}", shell_path(exe_cmd)),
        "timeout": 10
    });

    let hooks = root
        .as_object_mut()
        .expect("checked in read_settings")
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        bail!("{}: \"hooks\" must be an object", path.display());
    };
    let session = hooks.entry("SessionStart").or_insert_with(|| json!([]));
    let Some(groups) = session.as_array_mut() else {
        bail!("{}: \"hooks.SessionStart\" must be an array", path.display());
    };

    let mut found = false;
    for group in groups.iter_mut() {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            for h in handlers.iter_mut() {
                if is_our_handler(h) {
                    *h = handler.clone();
                    found = true;
                }
            }
        }
    }
    if !found {
        groups.push(json!({
            "matcher": "startup|resume|clear|compact",
            "hooks": [handler]
        }));
    }

    if existed && root == original {
        return Ok(Change::Unchanged);
    }
    write_settings(path, &root)?;
    Ok(if existed { Change::Updated } else { Change::Created })
}

/// Remove only our hook handler; delete the file if that leaves it empty.
pub fn remove_claude_hook(path: &Path) -> Result<Change> {
    use serde_json::Value;

    if !path.exists() {
        return Ok(Change::NotPresent);
    }
    let mut root = read_settings(path)?;

    let mut removed = false;
    if let Some(groups) = root
        .get_mut("hooks")
        .and_then(|h| h.get_mut("SessionStart"))
        .and_then(Value::as_array_mut)
    {
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = handlers.len();
                handlers.retain(|h| !is_our_handler(h));
                removed |= handlers.len() != before;
            }
        }
        // Only drop groups we emptied ourselves.
        if removed {
            groups.retain(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|h| !h.is_empty())
            });
        }
    }
    if !removed {
        return Ok(Change::NotPresent);
    }

    let obj = root.as_object_mut().expect("checked in read_settings");
    if let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
        if hooks.get("SessionStart").and_then(Value::as_array).is_some_and(|a| a.is_empty()) {
            hooks.remove("SessionStart");
        }
    }
    if obj.get("hooks").and_then(Value::as_object).is_some_and(|h| h.is_empty()) {
        obj.remove("hooks");
    }

    if obj.is_empty() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        write_settings(path, &root)?;
    }
    Ok(Change::Removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-rcl-setup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn upsert_is_idempotent_and_preserves_user_text() {
        let dir = tmp("idem");
        let f = dir.join("CLAUDE.md");
        fs::write(&f, "# My notes\n\nkeep me\n").unwrap();

        assert_eq!(upsert_marked_block(&f, MD_BEGIN, MD_END, "hello").unwrap(), Change::Updated);
        let first = fs::read_to_string(&f).unwrap();
        assert!(first.starts_with("# My notes\n\nkeep me\n"));
        assert!(first.contains("hello"));

        assert_eq!(upsert_marked_block(&f, MD_BEGIN, MD_END, "hello").unwrap(), Change::Unchanged);
        assert_eq!(fs::read_to_string(&f).unwrap(), first);

        assert_eq!(upsert_marked_block(&f, MD_BEGIN, MD_END, "changed").unwrap(), Change::Updated);
        let updated = fs::read_to_string(&f).unwrap();
        assert!(updated.contains("changed") && !updated.contains("hello"));
        assert_eq!(updated.matches(MD_BEGIN).count(), 1);
    }

    #[test]
    fn remove_restores_original_content() {
        let dir = tmp("remove");
        let f = dir.join("AGENTS.md");
        let original = "# Agents\n\nrules here\n";
        fs::write(&f, original).unwrap();

        upsert_marked_block(&f, MD_BEGIN, MD_END, "body").unwrap();
        assert_eq!(remove_marked_block(&f, MD_BEGIN, MD_END).unwrap(), Change::Removed);
        assert_eq!(fs::read_to_string(&f).unwrap(), original);
        assert_eq!(remove_marked_block(&f, MD_BEGIN, MD_END).unwrap(), Change::NotPresent);
    }

    #[test]
    fn remove_deletes_file_we_created() {
        let dir = tmp("created");
        let f = dir.join("GEMINI.md");
        assert_eq!(upsert_marked_block(&f, MD_BEGIN, MD_END, "body").unwrap(), Change::Created);
        remove_marked_block(&f, MD_BEGIN, MD_END).unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn unpaired_markers_abort_without_touching_the_file() {
        let dir = tmp("broken");
        let f = dir.join("CLAUDE.md");
        let broken = format!("intro\n{MD_BEGIN}\nhalf a block\n");
        fs::write(&f, &broken).unwrap();

        assert!(upsert_marked_block(&f, MD_BEGIN, MD_END, "x").is_err());
        assert!(remove_marked_block(&f, MD_BEGIN, MD_END).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), broken);
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let dir = tmp("crlf");
        let f = dir.join("CLAUDE.md");
        fs::write(&f, "line one\r\nline two\r\n").unwrap();
        upsert_marked_block(&f, MD_BEGIN, MD_END, "a\nb").unwrap();
        let out = fs::read_to_string(&f).unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "mixed line endings: {out:?}");
    }

    #[test]
    fn instruction_files_follow_target() {
        let base = Path::new("base");
        assert_eq!(instruction_files(base, false, "claude"), vec![base.join("CLAUDE.md")]);
        assert_eq!(instruction_files(base, false, "all").len(), 3);
        assert_eq!(
            instruction_files(base, true, "gemini"),
            vec![base.join(".gemini").join("GEMINI.md")]
        );
    }

    #[test]
    fn claude_hook_merges_without_touching_other_settings() {
        let dir = tmp("claude");
        let f = dir.join(".claude").join("settings.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let original = serde_json::json!({
            "model": "opus",
            "hooks": {
                "SessionStart": [
                    {"matcher": "startup", "hooks": [{"type": "command", "command": "echo mine"}]}
                ],
                "PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "fmt"}]}]
            }
        });
        fs::write(&f, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        assert_eq!(install_claude_hook(&f, "code-rcl").unwrap(), Change::Updated);
        assert_eq!(install_claude_hook(&f, "code-rcl").unwrap(), Change::Unchanged);
        let merged: serde_json::Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(merged["model"], "opus");
        assert_eq!(merged["hooks"]["PostToolUse"], original["hooks"]["PostToolUse"]);
        assert_eq!(merged["hooks"]["SessionStart"].as_array().unwrap().len(), 2);

        assert_eq!(remove_claude_hook(&f).unwrap(), Change::Removed);
        let restored: serde_json::Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn claude_hook_never_overwrites_invalid_json() {
        let dir = tmp("claude-bad");
        let f = dir.join("settings.json");
        fs::write(&f, "{ not json").unwrap();
        assert!(install_claude_hook(&f, "code-rcl").is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "{ not json");
    }

    #[test]
    fn git_hook_appends_to_existing_hook_and_refuses_foreign_interpreters() {
        let dir = tmp("hook");
        let hook = dir.join("post-commit");
        fs::write(&hook, "#!/bin/sh\necho existing\n").unwrap();
        install_git_hook(&hook, "C:\\bin\\code-rcl.exe").unwrap();
        let content = fs::read_to_string(&hook).unwrap();
        assert!(content.contains("echo existing"));
        assert!(content.contains("\"C:/bin/code-rcl.exe\" sync"));
        assert_eq!(remove_git_hook(&hook).unwrap(), Change::Removed);
        assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\necho existing\n");

        let py = dir.join("py-hook");
        fs::write(&py, "#!/usr/bin/env python\nprint(1)\n").unwrap();
        assert!(install_git_hook(&py, "code-rcl").is_err());
    }
}
