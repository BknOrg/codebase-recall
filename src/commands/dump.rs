use anyhow::Result;
use std::fs;

use crate::cli::DumpArgs;
use crate::dump::{formatter, walker};

pub fn run(args: DumpArgs) -> Result<()> {
    let (tree_paths, files) = walker::collect_files(&args.path, args.max_size_kb)?;
    let tree_view = formatter::build_tree_view(&tree_paths);
    let file_view = formatter::build_content_view(&tree_view, &files);

    fs::write(&args.output, &file_view)?;
    println!(
        "Codebase context successfully written to: {}",
        args.output.display()
    );

    Ok(())
}
