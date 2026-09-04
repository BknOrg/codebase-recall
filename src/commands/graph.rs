use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::cache::CacheDb;
use crate::cli::{GraphArgs, GraphQuery, SyncArgs};
use crate::commands::sync;
use crate::graph::render::{self, Format};
use crate::graph::resolve::{self, GraphOptions, Scope};
use crate::graph::CodeGraph;

/// Auto-sync (unless disabled) and resolve the graph selected by `query`.
/// Shared by `graph` (file output) and `serve` (browser).
pub fn build_graph(query: &GraphQuery) -> Result<CodeGraph> {
    let project = query.project.clone();
    let mut db = CacheDb::open(&project)?;

    if !query.no_sync {
        let sync_args = SyncArgs {
            project: project.clone(),
            max_file_kb: 512,
            language: Vec::new(),
        };
        let s = sync::sync_cache(&mut db, &sync_args)?;
        eprintln!(
            "auto-sync: +{} ~{} ={} -{} ({} symbols)",
            s.added, s.changed, s.unchanged, s.removed, s.symbols
        );
    }

    let scope = match query.scope.to_ascii_lowercase().as_str() {
        "file" => Scope::File,
        "symbol" => Scope::Symbol,
        "both" | "" => Scope::Both,
        other => anyhow::bail!("unknown --scope `{other}` (expected file, symbol, or both)"),
    };
    let kinds: HashSet<String> = query
        .kinds
        .iter()
        .map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| !k.is_empty())
        .collect();
    let path_glob = match &query.path {
        Some(p) => Some(resolve::compile_glob(p, &project)?),
        None => None,
    };

    let opts = GraphOptions {
        root: project.display().to_string(),
        scope,
        kinds,
        min_confidence: query.min_confidence,
        include_external: query.include_external,
        path_glob,
        focus: query.focus.clone(),
        depth: query.depth,
    };

    resolve::build(&db, &opts)
}

pub fn run(args: GraphArgs) -> Result<()> {
    let project = args.query.project.clone();

    let formats = parse_formats(&args.format)?;

    let graph = build_graph(&args.query)?;

    let targets = output_targets(&project, &args.output, &formats);
    for (format, path) in targets {
        let body = render::render(&graph, format)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {} ({} format)", path.display(), format.extension());
    }

    println!(
        "graph: {} nodes, {} edges",
        graph.nodes.len(),
        graph.edges.len()
    );
    Ok(())
}

fn parse_formats(tokens: &[String]) -> Result<Vec<Format>> {
    let mut out = Vec::new();
    for t in tokens {
        let f = Format::parse(t)?;
        if !out.contains(&f) {
            out.push(f);
        }
    }
    if out.is_empty() {
        out.push(Format::Html);
    }
    Ok(out)
}

fn output_targets(
    project: &std::path::Path,
    output: &Option<PathBuf>,
    formats: &[Format],
) -> Vec<(Format, PathBuf)> {
    match output {
        None => formats
            .iter()
            .map(|f| {
                (
                    *f,
                    project
                        .join(crate::cache::CODE_CTX_DIR)
                        .join(format!("code-graph.{}", f.extension())),
                )
            })
            .collect(),
        Some(p) if formats.len() == 1 => vec![(formats[0], p.clone())],
        Some(p) => {
            // Treat as a stem: strip a trailing known extension, then append each.
            let stem = match p.extension().and_then(|e| e.to_str()) {
                Some("html") | Some("json") | Some("dot") => p.with_extension(""),
                _ => p.clone(),
            };
            formats
                .iter()
                .map(|f| {
                    let mut path = stem.clone().into_os_string();
                    path.push(format!(".{}", f.extension()));
                    (*f, PathBuf::from(path))
                })
                .collect()
        }
    }
}
