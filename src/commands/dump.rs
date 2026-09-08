use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::cli::{DumpArgs, GraphQuery};
use crate::commands::graph::build_graph;
use crate::dump::{formatter, walker};

pub fn run(args: DumpArgs) -> Result<()> {
    let (tree_paths, files, metadata_header) = if let Some(focus_target) = &args.relation {
        let query = GraphQuery {
            project: args.path.clone(),
            scope: "both".to_string(),
            kinds: vec![
                "imports".to_string(),
                "calls".to_string(),
                "contains".to_string(),
            ],
            path: None,
            focus: Some(focus_target.clone()),
            depth: args.depth,
            min_confidence: 0.4,
            include_external: false,
            max_nodes: 4000,
            no_sync: args.no_sync,
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

        println!(
            "Focused dump: target '{}' (depth {}) -> {} connected files",
            focus_target,
            args.depth,
            target_rel_paths.len()
        );

        let (tree_paths, files) =
            walker::collect_related_files(&args.path, &target_rel_paths, args.max_size_kb);

        let header = format!(
            "> **Targeted dump**: focused on `{}` with depth {}. \n\n",
            focus_target, args.depth
        );

        (tree_paths, files, header)
    } else {
        let (tree_paths, files) = walker::collect_files(&args.path, args.max_size_kb)?;
        (tree_paths, files, String::new())
    };
    let tree_view = formatter::build_tree_view(&tree_paths);
    let mut file_view = formatter::build_content_view(&tree_view, &files);

    if !metadata_header.is_empty() {
        file_view = format!("{}{}", metadata_header, file_view);
    }

    fs::write(&args.output, &file_view)?;
    println!(
        "Codebase context successfully written to: {}",
        args.output.display()
    );

    Ok(())
}
