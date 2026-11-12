use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::cache::CacheDb;
use crate::cli::{GraphArgs, GraphQuery, SyncArgs};
use crate::commands::sync;
use crate::config::{self, ProjectConfig};
use crate::graph::CodeGraph;
use crate::graph::render::{self, Format};
use crate::graph::resolve::{self, GraphOptions, Scope};

/// Auto-sync (unless disabled) and resolve the graph selected by `query`.
/// Shared by `graph` (file output) and `serve` (browser).
pub fn build_graph(query: &GraphQuery) -> Result<CodeGraph> {
    query.precise.validate()?;
    let project = query.project.clone();
    let cfg = ProjectConfig::load(&project)?;
    let settings = config::resolve_graph(query, &cfg);
    let mut db = CacheDb::open(&project)?;

    if !query.no_sync {
        let sync_args = SyncArgs {
            json: false,
            project: project.clone(),
            max_file_kb: None,
            language: Vec::new(),
            no_report: true,
            precise: query.precise.clone(),
        };
        let s = sync::sync_cache(&mut db, &sync_args)?;
        eprintln!(
            "auto-sync: +{} ~{} ={} -{} ({} symbols)",
            s.added, s.changed, s.unchanged, s.removed, s.symbols
        );

        if query.precise.requested() {
            // Unlike `sync --precise`, a failure here is reported but does not
            // abort: the graph built from heuristic edges is still worth
            // rendering, and `report_precise` makes clear it is not the
            // compiler-grade one that was asked for.
            let stats = sync::run_precise(&mut db, &sync_args)?;
            sync::report_precise(&stats);
            if !stats.errors.is_empty()
                && !query.precise.allow_heuristic_fallback
                && !query.precise.precise_selective
            {
                anyhow::bail!("precise resolution failed: {}", stats.errors.join("; "));
            }
            if !stats.errors.is_empty() {
                crate::operation::partial();
            }
        }
    }

    let scope = match query.scope.to_ascii_lowercase().as_str() {
        "file" => Scope::File,
        "symbol" => Scope::Symbol,
        "both" | "" => Scope::Both,
        other => anyhow::bail!("unknown --scope `{other}` (expected file, symbol, or both)"),
    };
    let kinds: HashSet<String> = settings
        .kinds
        .iter()
        .map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| !k.is_empty())
        .collect();
    let path_glob = match &query.path {
        Some(p) => {
            let normalized_path = p.replace("\\", "/");
            Some(resolve::compile_glob(&normalized_path, &project)?)
        }
        None => None,
    };

    // Absolute path so the viewer can show the actual project folder name
    // (e.g. "code-reviewer") instead of "." when invoked without an explicit
    // path — this field is display-only (see resolve::compile_glob's unused
    // `_root` param), so canonicalizing it has no effect on graph resolution.
    let root_display = project
        .canonicalize()
        .unwrap_or_else(|_| project.clone())
        .display()
        .to_string();

    let opts = GraphOptions {
        root: root_display,
        scope,
        kinds,
        min_confidence: settings.min_confidence,
        include_external: settings.include_external,
        path_glob,
        focus: query.focus.clone(),
        depth: settings.depth,
        max_nodes: settings.max_nodes,
    };

    resolve::build(&db, &opts)
}

pub fn run(mut args: GraphArgs) -> Result<()> {
    let project = args.query.project.clone();
    ProjectConfig::load(&project)?.apply_precise_default(&mut args.query.precise);

    let formats = parse_formats(&args.format)?;

    let graph = build_graph(&args.query)?;

    let targets = output_targets(&project, &args.output, &formats);
    let has_stdout = targets.iter().any(|(_, p)| p.as_os_str() == "-");

    for (format, path) in targets {
        let body = if format == Format::Json {
            crate::output::json(&project, args.query.no_sync, &graph)?
        } else {
            render::render(&graph, format)?
        };
        if path.as_os_str() == "-" {
            println!("{body}");
            continue;
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {} ({} format)", path.display(), format.extension());
    }

    if has_stdout {
        eprintln!(
            "graph: {} nodes, {} edges",
            graph.nodes.len(),
            graph.edges.len()
        );
    } else {
        println!(
            "graph: {} nodes, {} edges",
            graph.nodes.len(),
            graph.edges.len()
        );
    }
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
        Some(p) if p.as_os_str() == "-" => {
            formats.iter().map(|f| (*f, PathBuf::from("-"))).collect()
        }
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
