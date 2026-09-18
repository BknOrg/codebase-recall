use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::cache::CacheDb;
use crate::cache::models::SymbolRow;

/// One project-relative, `/`-normalized new-file line range touched by the working tree diff.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedRange {
    pub path: String,
    /// 1-based, inclusive.
    pub start_line: i64,
    /// 1-based, inclusive.
    pub end_line: i64,
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .context("git not found on PATH; `impact --diff` requires git to detect changed symbols")?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `git diff` against HEAD in `project_root` and return changed line ranges
/// per file, relative to `project_root`. Returns `Ok(vec![])` if there are no
/// changes (not an error).
pub fn scan_diff(project_root: &Path) -> Result<Vec<ChangedRange>> {
    let toplevel = Command::new("git")
        .args(["-C", &project_root.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .context("git not found on PATH; `impact --diff` requires git to detect changed symbols")?;
    if !toplevel.status.success() {
        bail!(
            "'{}' is not inside a git repository (required for --diff)",
            project_root.display()
        );
    }
    let repo_root = Path::new(String::from_utf8_lossy(&toplevel.stdout).trim()).to_path_buf();

    let diff_out = match run_git(&repo_root, &["diff", "--unified=0", "HEAD"]) {
        Ok(s) => s,
        Err(e) => {
            bail!(
                "git repository at '{}' has no commits yet; --diff needs at least one commit to diff against ({e})",
                repo_root.display()
            );
        }
    };

    let project_root_abs = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let mut ranges = Vec::new();
    let mut current_path: Option<Option<String>> = None; // Some(None) = file deleted, skip

    for line in diff_out.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            if rest.trim() == "/dev/null" {
                current_path = Some(None);
                continue;
            }
            let raw = rest.strip_prefix("b/").unwrap_or(rest).trim();
            let rebased = rebase_to_project(&repo_root, &project_root_abs, raw);
            current_path = Some(rebased);
            continue;
        }

        if let Some((new_start, new_count)) = parse_hunk_header(line) {
            if new_count == 0 {
                continue; // pure deletion hunk, no new-file lines to map
            }
            if let Some(Some(path)) = &current_path {
                ranges.push(ChangedRange {
                    path: path.clone(),
                    start_line: new_start,
                    end_line: new_start + new_count - 1,
                });
            }
        }
    }

    Ok(ranges)
}

/// Rebase a git-relative path (relative to `repo_root`) to be relative to
/// `project_root`, `/`-normalized. Returns `None` if the path falls outside
/// `project_root` (not part of the synced project).
fn rebase_to_project(repo_root: &Path, project_root_abs: &Path, git_relative: &str) -> Option<String> {
    let abs = repo_root.join(git_relative);
    let abs = abs.canonicalize().unwrap_or(abs);
    let rel = abs.strip_prefix(project_root_abs).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Parse a unified-diff hunk header (`@@ -a,b +c,d @@...`) and return the
/// new-file `(start_line, line_count)`. `line_count` defaults to `1` when
/// omitted, per unified-diff convention.
fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    let rest = line.strip_prefix("@@ ")?;
    let plus_start = rest.find('+')?;
    let after_plus = &rest[plus_start + 1..];
    let end = after_plus.find(' ')?;
    let spec = &after_plus[..end];

    let mut parts = spec.splitn(2, ',');
    let start: i64 = parts.next()?.parse().ok()?;
    let count: i64 = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// Resolve each `ChangedRange` to the overlapping symbol(s) in the cache,
/// deduplicated by symbol id.
pub fn resolve_changed_symbols(db: &CacheDb, ranges: &[ChangedRange]) -> Result<Vec<SymbolRow>> {
    let mut seen = HashSet::new();
    let mut symbols = Vec::new();

    for range in ranges {
        let Some(file) = db.file_by_path(&range.path)? else {
            continue;
        };
        for sym in db.symbols_overlapping_lines(file.id, range.start_line, range.end_line)? {
            if seen.insert(sym.id) {
                symbols.push(sym);
            }
        }
    }

    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunk_header_with_explicit_count() {
        assert_eq!(parse_hunk_header("@@ -1,2 +3,4 @@ fn foo() {"), Some((3, 4)));
    }

    #[test]
    fn parses_hunk_header_with_omitted_count() {
        assert_eq!(parse_hunk_header("@@ -10,0 +15 @@"), Some((15, 1)));
    }

    #[test]
    fn parses_pure_deletion_hunk_as_zero_count() {
        assert_eq!(parse_hunk_header("@@ -5,2 +7,0 @@"), Some((7, 0)));
    }

    #[test]
    fn ignores_non_hunk_lines() {
        assert_eq!(parse_hunk_header("diff --git a/foo.rs b/foo.rs"), None);
        assert_eq!(parse_hunk_header("Binary files a/x and b/x differ"), None);
    }
}
