use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::models::SymbolRow;
use crate::cache::CacheDb;
use crate::cli::{DigestArgs, GraphQuery, PreciseArgs};
use crate::commands::graph::build_graph;

#[derive(Debug, Serialize)]
pub struct DigestReport {
    pub project: String,
    pub total_files: usize,
    pub total_symbols: usize,
    pub public_symbols: usize,
    pub languages: HashMap<String, usize>,
    pub hubs: Vec<HubItem>,
    pub modules: Vec<ModuleDigest>,
}

#[derive(Debug, Serialize)]
pub struct HubItem {
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub degree: u32,
}

#[derive(Debug, Serialize)]
pub struct ModuleDigest {
    pub directory: String,
    pub files: Vec<FileDigest>,
}

#[derive(Debug, Serialize)]
pub struct FileDigest {
    pub path: String,
    pub language: String,
    pub types: Vec<TypeDigest>,
    pub functions: Vec<SymbolItem>,
}

#[derive(Debug, Serialize)]
pub struct TypeDigest {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub methods: Vec<SymbolItem>,
}

#[derive(Debug, Serialize)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub line: Option<i64>,
    pub is_exported: bool,
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

pub fn run(args: DigestArgs) -> Result<()> {
    let (project_root, sub_prefix) = resolve_target(&args.path);

    let mut db = CacheDb::open(&project_root)
        .context("failed to open graph cache DB; run `code-rcl init` first")?;

    // Auto-sync if not disabled
    if !args.no_sync {
        let sync_args = crate::cli::SyncArgs {
            project: project_root.clone(),
            max_file_kb: 512,
            language: Vec::new(),
            precise: PreciseArgs::default(),
        };
        let _ = crate::commands::sync::sync_cache(&mut db, &sync_args);
    }

    let all_files = db.all_files()?;
    let all_symbols = db.all_symbols()?;

    // Compute top architecture hubs using relation graph
    let query = GraphQuery {
        project: project_root.clone(),
        scope: "both".to_string(),
        kinds: vec!["calls".to_string(), "imports".to_string()],
        path: None,
        focus: None,
        depth: 2,
        min_confidence: 0.0,
        include_external: false,
        max_nodes: 0,
        no_sync: true,
        precise: PreciseArgs::default(),
    };

    let hubs = if let Ok(graph) = build_graph(&query) {
        let mut sorted_nodes = graph.nodes;
        sorted_nodes.sort_by_key(|b| std::cmp::Reverse(b.degree.unwrap_or(0)));
        sorted_nodes
            .into_iter()
            .filter(|n| n.degree.unwrap_or(0) > 0 && n.kind != "file" && n.kind != "dir")
            .take(6)
            .map(|n| HubItem {
                label: n.label,
                kind: n.kind,
                path: n.path,
                degree: n.degree.unwrap_or(0),
            })
            .collect()
    } else {
        Vec::new()
    };

    let filtered_files: Vec<_> = all_files
        .into_iter()
        .filter(|f| {
            if sub_prefix.is_empty() {
                true
            } else {
                let norm = f.path.replace('\\', "/");
                norm == sub_prefix || norm.starts_with(&format!("{sub_prefix}/"))
            }
        })
        .collect();

    let mut languages: HashMap<String, usize> = HashMap::new();
    for f in &filtered_files {
        *languages.entry(f.language.clone()).or_insert(0) += 1;
    }

    let mut syms_by_file: HashMap<i64, Vec<&SymbolRow>> = HashMap::new();
    for s in &all_symbols {
        syms_by_file.entry(s.file_id).or_default().push(s);
    }

    let filtered_file_ids: HashSet<i64> = filtered_files.iter().map(|f| f.id).collect();
    let target_symbols: Vec<_> = all_symbols
        .iter()
        .filter(|s| filtered_file_ids.contains(&s.file_id))
        .collect();
    let total_symbols = target_symbols.len();
    let public_symbols = target_symbols.iter().filter(|s| s.is_exported).count();

    // Cache file content for signature extraction
    let mut file_lines_cache: HashMap<String, Vec<String>> = HashMap::new();

    // Group files by module directory
    let mut modules_map: BTreeMap<String, Vec<FileDigest>> = BTreeMap::new();

    for f in &filtered_files {
        let symbols = syms_by_file.get(&f.id).cloned().unwrap_or_default();
        if symbols.is_empty() && !args.all {
            continue;
        }

        // Read file content for signature extraction
        let abs_path = project_root.join(&f.path);
        let lines: &[String] = if let Some(cached) = file_lines_cache.get(&f.path) {
            cached
        } else {
            let loaded = fs::read_to_string(&abs_path)
                .map(|c| c.lines().map(String::from).collect())
                .unwrap_or_default();
            file_lines_cache.entry(f.path.clone()).or_insert(loaded)
        };

        // Categorize symbols into types, methods, and standalone functions
        let mut types_map: HashMap<String, TypeDigest> = HashMap::new();
        let mut type_names: HashSet<String> = HashSet::new();
        let mut standalone_functions: Vec<SymbolItem> = Vec::new();
        let mut methods: Vec<&SymbolRow> = Vec::new();

        for s in symbols {
            if !args.all && !s.is_exported {
                continue;
            }

            let sig = extract_signature(lines, s.start_line, s.end_line);

            match s.kind.as_str() {
                "struct" | "enum" | "class" | "interface" | "trait" | "type" => {
                    type_names.insert(s.name.clone());
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
                    });
                }
                _ => {}
            }
        }

        // Attach methods to their enclosing types or keep as functions
        for m in methods {
            let sig = extract_signature(lines, m.start_line, m.end_line);
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
            };

            if let Some(parent_type) = &m.type_name
                && let Some(td) = types_map.get_mut(parent_type)
            {
                td.methods.push(item);
                continue;
            }
            standalone_functions.push(item);
        }

        let mut types: Vec<TypeDigest> = types_map.into_values().collect();
        types.sort_by(|a, b| a.name.cmp(&b.name));
        standalone_functions.sort_by_key(|a| a.line);

        if types.is_empty() && standalone_functions.is_empty() && !args.all {
            continue;
        }

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

    let report = DigestReport {
        project: project_name,
        total_files: filtered_files.len(),
        total_symbols,
        public_symbols,
        languages,
        hubs,
        modules,
    };

    if args.json {
        let json_text = serde_json::to_string_pretty(&report)?;
        if let Some(out_path) = &args.output {
            fs::write(out_path, &json_text)?;
            eprintln!("wrote {}", out_path.display());
        } else {
            println!("{json_text}");
        }
    } else {
        let md = format_markdown_digest(&report);
        if let Some(out_path) = &args.output {
            fs::write(out_path, &md)?;
            eprintln!("wrote {}", out_path.display());
        } else {
            println!("{md}");
        }
    }

    Ok(())
}

fn extract_signature(lines: &[String], start_line: Option<i64>, end_line: Option<i64>) -> String {
    let Some(start) = start_line else {
        return String::new();
    };
    let start_idx = (start.max(1) - 1) as usize;
    if start_idx >= lines.len() {
        return String::new();
    }

    let end_idx = match end_line {
        Some(end) => (end.max(start) - 1) as usize,
        None => start_idx,
    };

    let mut sig = String::new();
    let max_lines = (end_idx - start_idx + 1).min(5);

    for i in 0..max_lines {
        let idx = start_idx + i;
        if idx >= lines.len() {
            break;
        }
        let line = lines[idx].trim();
        if line.starts_with("//") || line.starts_with('#') || line.starts_with("/*") {
            continue;
        }

        if !sig.is_empty() {
            sig.push(' ');
        }

        if let Some(open) = line.find('{') {
            sig.push_str(line[..open].trim());
            break;
        } else if let Some(semi) = line.find(';') {
            sig.push_str(line[..semi].trim());
            break;
        } else if line.ends_with(':') {
            sig.push_str(line.trim_end_matches(':').trim());
            break;
        } else {
            sig.push_str(line);
        }
    }

    sig.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_markdown_digest(report: &DigestReport) -> String {
    let mut md = String::new();

    md.push_str(&format!("# Architecture Digest: {}\n\n", report.project));

    let mut langs: Vec<_> = report.languages.iter().collect();
    langs.sort_by(|a, b| b.1.cmp(a.1));
    let lang_summary = langs
        .iter()
        .map(|(l, count)| format!("{l} ({count})"))
        .collect::<Vec<_>>()
        .join(", ");

    md.push_str(&format!(
        "**Summary**: {} files | {} symbols ({} public) | Languages: {}\n\n",
        report.total_files,
        report.total_symbols,
        report.public_symbols,
        if lang_summary.is_empty() { "none".to_string() } else { lang_summary }
    ));

    if !report.hubs.is_empty() {
        md.push_str("## Core Architecture Hubs\n\n");
        for hub in &report.hubs {
            let loc = hub
                .path
                .as_deref()
                .map(|p| format!(" (`{p}`)"))
                .unwrap_or_default();
            md.push_str(&format!(
                "- **`{}`**{} — {} connections [{}]\n",
                hub.label, loc, hub.degree, hub.kind
            ));
        }
        md.push_str("\n---\n\n");
    }

    md.push_str("## Modules & Public APIs\n\n");

    for m in &report.modules {
        md.push_str(&format!("### Directory: `{}`\n\n", m.directory));

        for f in &m.files {
            md.push_str(&format!("#### `{}` ({})\n", f.path, f.language));

            for t in &f.types {
                md.push_str(&format!("- **`{}`**\n", t.signature));
                for method in &t.methods {
                    md.push_str(&format!("  - `{}`\n", method.signature));
                }
            }

            if !f.functions.is_empty() {
                if !f.types.is_empty() {
                    md.push_str("- **Functions**:\n");
                    for func in &f.functions {
                        md.push_str(&format!("  - `{}`\n", func.signature));
                    }
                } else {
                    for func in &f.functions {
                        md.push_str(&format!("- `{}`\n", func.signature));
                    }
                }
            }
            md.push('\n');
        }
    }

    md
}
