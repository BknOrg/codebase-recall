use ignore::{DirEntry, WalkBuilder};
use std::path::{Path, PathBuf};

const IGNORED_NAMES_FOR_CONTENT: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "poetry.lock",
    "composer.lock",
    "go.sum",
    ".DS_Store",
    "Thumbs.db",
    "LICENSE",
];

const IGNORED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "vendor",
    ".git",
    ".svn",
    ".hg",
    ".cache",
    "dist",
    ".gradle",
    ".dart_tool",
];

const IGNORED_EXTENSIONS_FOR_CONTENT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "svg", "woff", "woff2", "ttf", "otf", "pdf", "zip",
    "tar", "gz", "map", "sqlite", "db", "bin", "exe", "dll", "so", "dylib", "key", "pem", "crt",
    "cer", "lock", "md",
];

const MINIFIED_EXTENSIONS: &[&str] = &[
    ".min.js",
    ".min.css",
    ".min.ts",
    ".min.bundles.js",
    ".chunk.js",
    ".d.ts.map",
];

pub struct FileEntry {
    pub relative_path: PathBuf,
    pub content: String,
}

fn is_ignored_dir(entry: &DirEntry) -> bool {
    if entry.file_type().is_some_and(|ft| ft.is_dir()) {
        if let Some(name) = entry.file_name().to_str() {
            if IGNORED_DIRECTORIES.contains(&name) {
                return false;
            }
        }
    }
    true
}

fn should_skip_content(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let lower_name = file_name.to_lowercase();

    if lower_name.starts_with(".env") || IGNORED_NAMES_FOR_CONTENT.contains(&file_name) {
        return true;
    }

    if MINIFIED_EXTENSIONS
        .iter()
        .any(|ext| lower_name.ends_with(ext))
    {
        return true;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if IGNORED_EXTENSIONS_FOR_CONTENT.contains(&ext.to_lowercase().as_str()) {
            return true;
        }
    }

    false
}

/// Walk `root` with the same ignore rules as [`collect_files`] and return the
/// absolute path of every regular file (no size or extension filtering).
pub fn collect_source_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .filter_entry(is_ignored_dir)
        .build();

    for result in walker {
        let entry = result?;
        let path = entry.path();
        if path.is_file() {
            paths.push(path.to_path_buf());
        }
    }

    Ok(paths)
}

pub fn collect_related_files(
    root: &Path,
    relative_path: &[PathBuf],
    max_size_kb: u64,
) -> (Vec<PathBuf>, Vec<FileEntry>) {
    let max_byte = max_size_kb * 1024;
    let mut tree_paths = Vec::new();
    let mut files = Vec::new();

    for rel in relative_path {
        let abs = root.join(rel);
        if !abs.is_file() {
            continue;
        }

        tree_paths.push(rel.clone());

        if let Ok(metadata) = abs.metadata() {
            if metadata.len() > max_byte {
                continue;
            }
        }
        if let Ok(content) = std::fs::read_to_string(&abs) {
            files.push(FileEntry {
                relative_path: rel.clone(),
                content,
            });
        }
    }
    tree_paths.sort();
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    (tree_paths, files)
}

pub fn collect_files(
    root: &Path,
    max_size_kb: u64,
) -> anyhow::Result<(Vec<PathBuf>, Vec<FileEntry>)> {
    let mut tree_paths = Vec::new();
    let mut file_entries = Vec::new();
    let max_bytes = max_size_kb * 1024;

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .filter_entry(is_ignored_dir)
        .build();

    for result in walker {
        let entry = result?;
        let path = entry.path();

        if path == root {
            continue;
        }

        let rel_path = path.strip_prefix(root)?.to_path_buf();
        tree_paths.push(rel_path.clone());

        if path.is_file() && !should_skip_content(path) {
            if let Ok(metadata) = std::fs::metadata(path) {
                if metadata.len() <= max_bytes {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        file_entries.push(FileEntry {
                            relative_path: rel_path,
                            content,
                        });
                    }
                }
            }
        }
    }

    Ok((tree_paths, file_entries))
}
