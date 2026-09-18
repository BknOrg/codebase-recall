pub mod extractor;
pub mod markdown;
pub mod models;

pub use extractor::*;
pub use markdown::*;
pub use models::*;

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::CacheDb;
use crate::cache::models::SymbolRow;
use crate::cli::{DigestArgs, PreciseArgs};
use crate::commands::graph::build_graph;

pub(crate) fn ensure_extension(path: &Path, json: bool) -> PathBuf {
    let ext = if json { "json" } else { "md" };
    let mut normalized = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("digest");

    if !file_name.to_lowercase().ends_with(&format!(".{ext}")) {
        normalized.set_file_name(format!("{file_name}.{ext}"));
    }
    normalized
}

fn resolve_target(path: &Path) -> (PathBuf, String) {
    if path.is_file() {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut current = parent;
        let mut root = parent.to_path_buf();
        loop {
            if current.join(".code-rcl").exists()
                || current.join("Cargo.toml").exists()
                || current.join("package.json").exists()
                || current.join("pyproject.toml").exists()
            {
                root = current.to_path_buf();
                break;
            }
            match current.parent() {
                Some(p) if p != current => current = p,
                _ => break,
            }
        }
        let sub = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        (root, sub)
    } else if path == Path::new(".") && !Path::new(".code-rcl").exists() {
        if let Ok(abs_current) = std::env::current_dir() {
            let mut p = abs_current.as_path();
            while let Some(parent) = p.parent() {
                if parent.join(".code-rcl").exists() {
                    let sub = abs_current
                        .strip_prefix(parent)
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    return (parent.to_path_buf(), sub);
                }
                p = parent;
            }
        }
        (PathBuf::from("."), String::new())
    } else {
        (path.to_path_buf(), String::new())
    }
}

fn resolve_root(args: &DigestArgs) -> (PathBuf, String) {
    match &args.project {
        Some(explicit) => {
            let sub = if args.path == Path::new(".") {
                String::new()
            } else if args.path.is_relative() {
                args.path.to_string_lossy().replace('\\', "/")
            } else {
                args.path
                    .strip_prefix(explicit)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default()
            };
            (explicit.clone(), sub)
        }
        None => resolve_target(&args.path),
    }
}

pub fn run(args: DigestArgs) -> Result<()> {
    let report = generate_digest(&args)?;

    let content = if args.json {
        serde_json::to_string_pretty(&report)?
    } else {
        format_markdown_digest(&report)
    };

    if let Some(out_path) = &args.output {
        let out_path = ensure_extension(out_path, args.json);
        fs::write(&out_path, &content)
            .with_context(|| format!("failed to write digest to {}", out_path.display()))?;
        eprintln!(
            "Digest successfully written to {} ({} files, {} symbols)",
            out_path.display(),
            report.total_files,
            report.total_symbols
        );
    } else {
        print!("{content}");
    }

    Ok(())
}

fn compute_hubs(project_root: &Path, no_sync: bool) -> Vec<HubItem> {
    let query = crate::service::analysis_query(
        project_root,
        crate::service::call_and_import_kinds(),
        no_sync,
        PreciseArgs::default(),
    );


    let Ok(graph) = build_graph(&query) else {
        return Vec::new();
    };
    hubs_from_graph(&graph)
}

/// Accessors and trait plumbing that every type has; their degree says how often
/// a name is reused, not where the architecture centres.
const TRIVIAL_HUB_NAMES: &[&str] = &[
    "as_str", "as_ref", "as_mut", "new", "default", "fmt", "from", "into", "clone", "eq", "hash",
    "drop", "len", "is_empty", "get", "set", "iter", "next", "to_string", "text", "line", "push", "pop", "name",
];

fn is_trivial_hub(n: &crate::graph::Node) -> bool {
    n.kind == "method" && TRIVIAL_HUB_NAMES.contains(&n.label.as_str())
}

/// The ten most-connected nodes (degree >= 3), most connected first.
pub fn hubs_from_graph(graph: &crate::graph::CodeGraph) -> Vec<HubItem> {
    let mut degree: HashMap<&str, u32> = HashMap::new();
    for e in &graph.edges {
        *degree.entry(&e.source).or_default() += 1;
        *degree.entry(&e.target).or_default() += 1;
    }

    let mut hubs: Vec<(&crate::graph::Node, u32)> = graph
        .nodes
        .iter()
        .filter_map(|n| degree.get(n.id.as_str()).map(|&d| (n, d)))
        .filter(|(n, d)| *d >= 3 && n.kind != "directory" && !is_trivial_hub(n))
        .collect();

    hubs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.label.cmp(&b.0.label)));
    hubs.truncate(10);

    hubs.into_iter()
        .map(|(node, deg)| HubItem {
            label: node.label.clone(),
            kind: node.kind.clone(),
            path: node.path.clone(),
            degree: deg,
        })
        .collect()
}

pub fn generate_digest(args: &DigestArgs) -> Result<DigestReport> {
    let (project_root, sub_prefix) = resolve_root(args);

    if !args.no_sync {
        let mut db_mut = CacheDb::open(&project_root)?;
        let _ = crate::service::auto_sync(&mut db_mut, &project_root);
    }

    let db = CacheDb::open(&project_root)?;
    let all_files = db.all_files()?;
    let all_symbols = db.all_symbols()?;
    let mut symbols_by_file: HashMap<i64, Vec<SymbolRow>> = HashMap::new();
    for s in all_symbols {
        symbols_by_file.entry(s.file_id).or_default().push(s);
    }

    let filtered_files: Vec<_> = all_files
        .into_iter()
        .filter(|f| {
            if sub_prefix.is_empty() {
                true
            } else {
                let norm_path = f.path.replace('\\', "/");
                norm_path.starts_with(&sub_prefix)
            }
        })
        .collect();

    let mut languages: HashMap<String, usize> = HashMap::new();
    for f in &filtered_files {
        *languages.entry(f.language.clone()).or_default() += 1;
    }

    let hubs = compute_hubs(&project_root, true);

    let mut modules_map: BTreeMap<String, Vec<FileDigest>> = BTreeMap::new();
    let mut total_symbols = 0;
    let mut public_symbols = 0;

    for f in &filtered_files {
        let abs_path = project_root.join(&f.path);
        let content = fs::read_to_string(&abs_path).unwrap_or_default();
        let lines: Vec<String> = content.lines().map(String::from).collect();

        let symbols = symbols_by_file.remove(&f.id).unwrap_or_default();

        let mut type_names: HashSet<String> = HashSet::new();
        let mut types_map: BTreeMap<String, TypeDigest> = BTreeMap::new();
        let mut standalone_functions: Vec<SymbolItem> = Vec::new();
        let mut methods: Vec<&SymbolRow> = Vec::new();
        let effective_doc_lines = args.effective_doc_lines();

        for s in &symbols {
            // Same symbol kinds `report` counts, so the two summaries agree
            // whatever the `--all` filter hides from the listing below.
            if matches!(
                s.kind.as_str(),
                "struct" | "enum" | "class" | "interface" | "trait" | "type" | "method" | "function"
            ) {
                total_symbols += 1;
                if s.is_exported {
                    public_symbols += 1;
                }
            }
            if !args.all && !s.is_exported {
                continue;
            }

            let sig = extract_signature(&lines, s.start_line, s.end_line);
            let (doc, has_diagram) = extract_docs_and_diagram(
                &lines,
                s.start_line,
                s.end_line,
                &f.language,
                effective_doc_lines,
            );

            match s.kind.as_str() {
                "struct" | "enum" | "class" | "interface" | "trait" | "type" => {
                    type_names.insert(s.name.clone());
                    let has_diagram_in_fields =
                        check_body_has_diagram(&lines, s.start_line, s.end_line);
                    let combined_has_diagram = has_diagram || has_diagram_in_fields;
                    types_map.insert(
                        s.name.clone(),
                        TypeDigest {
                            name: s.name.clone(),
                            kind: s.kind.clone(),
                            signature: if sig.is_empty() {
                                format!("{} {}", s.kind, s.name)
                            } else {
                                sig
                            },
                            doc,
                            has_diagram: combined_has_diagram,
                            has_diagram_in_fields,
                            methods: Vec::new(),
                        },
                    );
                }
                "method" => {
                    methods.push(s);
                }
                "function" => {
                    standalone_functions.push(SymbolItem {
                        name: s.name.clone(),
                        kind: s.kind.clone(),
                        signature: if sig.is_empty() {
                            format!("fn {}", s.name)
                        } else {
                            sig
                        },
                        line: s.start_line,
                        is_exported: s.is_exported,
                        doc,
                        has_diagram,
                    });
                }
                _ => {}
            }
        }

        for m in methods {
            let sig = extract_signature(&lines, m.start_line, m.end_line);
            let (doc, has_diagram) = extract_docs_and_diagram(
                &lines,
                m.start_line,
                m.end_line,
                &f.language,
                effective_doc_lines,
            );
            let item = SymbolItem {
                name: m.name.clone(),
                kind: m.kind.clone(),
                signature: if sig.is_empty() {
                    format!("fn {}", m.name)
                } else {
                    sig
                },
                line: m.start_line,
                is_exported: m.is_exported,
                doc,
                has_diagram,
            };

            if let Some(tname) = &m.type_name {
                if let Some(td) = types_map.get_mut(tname) {
                    td.methods.push(item);
                    continue;
                }
            }
            standalone_functions.push(item);
        }

        let types: Vec<TypeDigest> = types_map.into_values().collect();

        let dir = Path::new(&f.path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .replace('\\', "/");

        let dir_key = if dir.is_empty() { ".".to_string() } else { dir };

        modules_map.entry(dir_key).or_default().push(FileDigest {
            path: f.path.replace('\\', "/"),
            language: f.language.clone(),
            types,
            functions: standalone_functions,
        });
    }

    let modules: Vec<ModuleDigest> = modules_map
        .into_iter()
        .map(|(directory, files)| ModuleDigest { directory, files })
        .collect();

    let project_name = project_root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "codebase".to_string());

    Ok(DigestReport {
        project: project_name,
        total_files: filtered_files.len(),
        total_symbols,
        public_symbols,
        languages,
        hubs,
        modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{CodeGraph, Edge, Node};

    fn node(id: &str, kind: &str, label: &str) -> Node {
        Node {
            id: id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            path: None,
            dir: None,
            language: None,
            exported: true,
            lines: None,
            degree: None,
            community: None,
        }
    }

    fn edge(source: &str, target: &str) -> Edge {
        Edge {
            source: source.to_string(),
            target: target.to_string(),
            kind: "calls".to_string(),
            confidence: 0.9,
            external: None,
            line: None,
        }
    }

    #[test]
    fn hubs_skip_trivial_accessor_methods() {
        let mut nodes = vec![node("as_str", "method", "as_str"), node("build", "function", "build")];
        let mut edges = Vec::new();
        for i in 0..4 {
            let caller = format!("c{i}");
            nodes.push(node(&caller, "function", &caller));
            edges.push(edge(&caller, "as_str"));
            edges.push(edge(&caller, "build"));
        }
        let graph = CodeGraph {
            version: 2,
            root: String::new(),
            generated_at: 0,
            nodes,
            edges,
            communities: Vec::new(),
        };
        let labels: Vec<_> = hubs_from_graph(&graph).into_iter().map(|h| h.label).collect();
        assert_eq!(labels, vec!["build"]);
    }
}
