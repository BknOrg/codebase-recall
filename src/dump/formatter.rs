use crate::dump::walker::FileEntry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct TreeNode {
    children: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    fn insert(&mut self, path: &Path) {
        let mut current = self;
        for component in path.iter() {
            let name = component.to_string_lossy().to_string();
            current = current.children.entry(name).or_default();
        }
    }

    fn render(&self, output: &mut String, prefix: &str) {
        let total = self.children.len();
        for (i, (name, node)) in self.children.iter().enumerate() {
            let is_last = i + 1 == total;
            let branch = if is_last { "└── " } else { "├── " };
            let next_prefix = if is_last { "    " } else { "│   " };

            output.push_str(prefix);
            output.push_str(branch);
            output.push_str(name);
            output.push('\n');

            node.render(output, &format!("{prefix}{next_prefix}"));
        }
    }
}

fn calculate_fence(content: &str) -> String {
    let mut max_streak = 0;
    let mut current_streak = 0;

    for ch in content.chars() {
        if ch == '`' {
            current_streak += 1;
            if current_streak > max_streak {
                max_streak = current_streak;
            }
        } else {
            current_streak = 0;
        }
    }

    let fence_len = if max_streak >= 3 { max_streak + 1 } else { 3 };
    "`".repeat(fence_len)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn build_tree_view(paths: &[PathBuf]) -> String {
    let mut root = TreeNode::default();
    for path in paths {
        root.insert(path);
    }

    let mut tree = String::new();
    root.render(&mut tree, "");
    tree
}

pub fn build_content_view(tree: &str, files: &[FileEntry]) -> String {
    let mut output = String::new();

    output.push_str("# Directory Tree\n\n");
    output.push_str("```text\n");
    output.push_str(tree);
    output.push_str("```\n\n");
    output.push_str("---\n\n");

    output.push_str("# Source Files\n\n");
    for file in files {
        let clean_path = normalize_path(&file.relative_path);
        let extension = file
            .relative_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let fence = calculate_fence(&file.content);

        output.push_str(&format!("## File: `{clean_path}`\n\n"));
        output.push_str(&format!("{fence}{extension}\n"));
        output.push_str(&file.content);
        if !file.content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!("{fence}\n\n"));
    }

    output
}
