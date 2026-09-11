use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{DumpArgs, GraphQuery};
use crate::commands::graph::build_graph;
use crate::dump::{formatter, walker};

fn ensure_md_extension(path: &PathBuf) -> PathBuf {
    let mut normalized = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("codebase-context");

    if !file_name.to_lowercase().ends_with(".md") {
        normalized.set_file_name(format!("{file_name}.md"));
    }
    normalized
}

pub fn run(args: DumpArgs) -> Result<()> {
    let output_file = ensure_md_extension(&args.output);

    if let Some(focus_target) = &args.relation {
        let (path, n) = relation_bundle(
            &args.path,
            focus_target,
            args.depth,
            args.max_size_kb,
            args.no_sync,
            &args.output,
        )?;
        println!(
            "Focused dump: target '{}' (depth {}) -> {} connected files",
            focus_target, args.depth, n
        );
        println!(
            "Codebase context successfully written to: {}",
            path.display()
        );
        return Ok(());
    }

    let (tree_paths, files) = walker::collect_files(&args.path, args.max_size_kb)?;
    let tree_view = formatter::build_tree_view(&tree_paths);
    let file_view = formatter::build_content_view(&tree_view, &files);
    fs::write(&output_file, &file_view)?;
    println!(
        "Codebase context successfully written to: {}",
        output_file.display()
    );
    Ok(())
}

/// Write a relation-aware ("dump -r") context bundle for `target` (a symbol name
/// or a project-relative file path) and everything within `depth` graph hops, to
/// `output`. Returns `(output_path, connected_file_count)`. Reused by the viewer's
/// `/dump` endpoint in `code-rcl serve`.
pub fn relation_bundle(
    project: &Path,
    target: &str,
    depth: u32,
    max_size_kb: u64,
    no_sync: bool,
    output: &Path,
) -> Result<(PathBuf, usize)> {
    let query = GraphQuery {
        project: project.to_path_buf(),
        scope: "both".to_string(),
        kinds: vec![
            "imports".to_string(),
            "calls".to_string(),
            "contains".to_string(),
        ],
        path: None,
        focus: Some(target.to_string()),
        depth,
        min_confidence: 0.4,
        include_external: false,
        max_nodes: 4000,
        no_sync,
    };

    let graph = build_graph(&query)?;
    let target_rel_paths: Vec<PathBuf> = graph
        .nodes
        .iter()
        .filter_map(|n| n.path.as_deref())
        .map(PathBuf::from)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let (tree_paths, files) =
        walker::collect_related_files(project, &target_rel_paths, max_size_kb);
    let tree_view = formatter::build_tree_view(&tree_paths);
    let body = formatter::build_content_view(&tree_view, &files);
    let header = format!(
        "> **Targeted dump**: focused on `{}` with depth {}. \n\n",
        target, depth
    );
    fs::write(output, format!("{header}{body}"))?;
    Ok((output.to_path_buf(), files.len()))
}
