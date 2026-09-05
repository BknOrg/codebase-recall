//! Turn cached files/symbols/imports/refs into a resolved [`CodeGraph`].

mod imports;
mod postprocess;
mod refs;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use globset::GlobMatcher;

use crate::cache::models::{FileRow, SymbolRow};
use crate::cache::CacheDb;
use crate::graph::{external_id, file_id, symbol_id, CodeGraph, Edge, Node};

use imports::{external_root, resolve_import};
use postprocess::{
    apply_focus, collapse_to_files, degree_map, enforce_max_nodes, prune_unreferenced_externals,
    rollup_directories,
};
use refs::resolve_ref;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    File,
    Symbol,
    Both,
}

pub struct GraphOptions {
    pub root: String,
    pub scope: Scope,
    pub kinds: HashSet<String>,
    pub min_confidence: f32,
    pub include_external: bool,
    pub path_glob: Option<GlobMatcher>,
    pub focus: Option<String>,
    pub depth: u32,
    /// Cap on total node count. Past this the lowest-degree symbol nodes are
    /// dropped (files/dirs/externals are always kept). `0` disables the cap.
    pub max_nodes: usize,
}

pub fn build(db: &CacheDb, opts: &GraphOptions) -> Result<CodeGraph> {
    let files = db.all_files()?;
    let symbols = db.all_symbols()?;
    let imports = db.all_imports()?;
    let refs = db.all_refs()?;

    let file_by_id: HashMap<i64, &FileRow> = files.iter().map(|f| (f.id, f)).collect();
    let path_set: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let frow_by_path: HashMap<&str, &FileRow> =
        files.iter().map(|f| (f.path.as_str(), f)).collect();

    let sym_by_id: HashMap<i64, &SymbolRow> = symbols.iter().map(|s| (s.id, s)).collect();
    let mut syms_by_file: HashMap<i64, Vec<&SymbolRow>> = HashMap::new();
    for s in &symbols {
        syms_by_file.entry(s.file_id).or_default().push(s);
    }

    // Does this file pass the --path glob?
    let keep_file = |path: &str| opts.path_glob.as_ref().map_or(true, |g| g.is_match(path));

    // ---- nodes -------------------------------------------------------------
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_ids: HashSet<String> = HashSet::new();
    // sym db id -> (node id string, owning file node id string)
    let mut sym_node: HashMap<i64, (String, String)> = HashMap::new();

    for f in &files {
        if !keep_file(&f.path) {
            continue;
        }
        let id = file_id(&f.path);
        node_ids.insert(id.clone());
        nodes.push(Node {
            id,
            kind: "file".into(),
            label: f.path.clone(),
            path: Some(f.path.clone()),
            dir: parent_dir(&f.path),
            language: Some(f.language.clone()),
            exported: true,
            lines: None,
            degree: None,
        });
    }

    let want_symbols = opts.scope != Scope::File;
    for s in &symbols {
        let Some(f) = file_by_id.get(&s.file_id) else {
            continue;
        };
        if !keep_file(&f.path) {
            continue;
        }
        let start = s.start_line.unwrap_or(0);
        let sid = symbol_id(&f.path, &s.name, start);
        sym_node.insert(s.id, (sid.clone(), file_id(&f.path)));
        if want_symbols {
            node_ids.insert(sid.clone());
            nodes.push(Node {
                id: sid,
                kind: s.kind.clone(),
                label: s.name.clone(),
                path: Some(f.path.clone()),
                dir: parent_dir(&f.path),
                language: Some(f.language.clone()),
                exported: s.is_exported,
                lines: Some([start, s.end_line.unwrap_or(start)]),
                degree: None,
            });
        }
    }

    // ---- edge accumulator ------------------------------------------------
    let mut edges: EdgeSet = EdgeSet::default();

    // contains
    if opts.kinds.contains("contains") && want_symbols {
        for s in &symbols {
            let Some((child, file_node)) = sym_node.get(&s.id) else {
                continue;
            };
            let parent = match s.parent_symbol_id.and_then(|pid| sym_node.get(&pid)) {
                Some((p, _)) => p.clone(),
                None => file_node.clone(),
            };
            edges.add(parent, child.clone(), "contains", 1.0, None);
        }
    }

    // ---- imports + symbol bindings -------------------------------------
    // (importer file id, local name) -> resolved target symbol db id
    let mut binding: HashMap<(i64, String), i64> = HashMap::new();

    for im in &imports {
        let Some(importer) = file_by_id.get(&im.file_id) else {
            continue;
        };
        let resolved = resolve_import(&importer.language, &importer.path, im, &path_set);

        if let Some(target_path) = resolved.as_deref() {
            if target_path != importer.path
                && opts.kinds.contains("imports")
                && keep_file(&importer.path)
                && keep_file(target_path)
            {
                edges.add(
                    file_id(&importer.path),
                    file_id(target_path),
                    "imports",
                    1.0,
                    None,
                );
            }
            // symbol binding for call resolution
            if let (Some(name), Some(tf)) = (
                im.imported_name.as_deref(),
                resolved.as_deref().and_then(|p| frow_by_path.get(p)),
            ) {
                if let Some(list) = syms_by_file.get(&tf.id) {
                    let hit = list
                        .iter()
                        .find(|s| s.name == name && s.is_exported)
                        .or_else(|| list.iter().find(|s| s.name == name));
                    if let Some(s) = hit {
                        let local = im.alias.clone().unwrap_or_else(|| name.to_string());
                        binding.insert((im.file_id, local), s.id);
                    }
                }
            }
        } else if opts.include_external && opts.kinds.contains("imports") && keep_file(&importer.path)
        {
            let label = external_root(&im.raw_specifier, &importer.language);
            let ext = external_id(&label);
            if node_ids.insert(ext.clone()) {
                nodes.push(Node {
                    id: ext.clone(),
                    kind: "external".into(),
                    label,
                    path: None,
                    dir: None,
                    language: None,
                    exported: false,
                    lines: None,
                    degree: None,
                });
            }
            edges.add(
                file_id(&importer.path),
                ext,
                "imports",
                1.0,
                Some(im.raw_specifier.clone()),
            );
        }
    }

    // ---- calls / references -------------------------------------------
    let mut defs_by_name: HashMap<&str, Vec<&SymbolRow>> = HashMap::new();
    for s in &symbols {
        defs_by_name.entry(s.name.as_str()).or_default().push(s);
    }

    for rf in &refs {
        let edge_kind = if rf.ref_kind == "call" {
            "calls"
        } else {
            "references"
        };
        if !opts.kinds.contains(edge_kind) {
            continue;
        }
        let Some(importer) = file_by_id.get(&rf.file_id) else {
            continue;
        };
        if !keep_file(&importer.path) {
            continue;
        }

        let src = match rf.from_symbol_id.and_then(|id| sym_node.get(&id)) {
            Some((s, _)) if want_symbols => s.clone(),
            _ => file_id(&importer.path),
        };

        let Some((target, confidence)) = resolve_ref(
            rf,
            importer,
            &syms_by_file,
            &binding,
            &defs_by_name,
            &file_by_id,
        ) else {
            continue;
        };
        if confidence < opts.min_confidence {
            continue;
        }
        let Some((tgt, tgt_file)) = sym_node.get(&target) else {
            continue;
        };
        let tgt = if want_symbols { tgt.clone() } else { tgt_file.clone() };
        if tgt == src {
            continue;
        }
        edges.add(src, tgt, edge_kind, confidence, None);
    }

    let _ = sym_by_id; // reserved for future receiver-type resolution

    // ---- finalize ------------------------------------------------------
    let mut edges = edges.into_vec();

    if opts.scope == Scope::File {
        collapse_to_files(&mut nodes, &mut edges, &sym_node);
    }

    if let Some(focus) = &opts.focus {
        apply_focus(&mut nodes, &mut edges, focus, opts.depth);
    }

    prune_unreferenced_externals(&mut nodes, &edges, opts.include_external);

    // Keep the node count in check, then annotate every node with its relation
    // degree and add the directory roll-up nodes the viewer drills down from.
    {
        let degree = degree_map(&edges);
        enforce_max_nodes(&mut nodes, &mut edges, &degree, opts.max_nodes);
    }
    let degree = degree_map(&edges);
    for n in &mut nodes {
        n.degree = Some(degree.get(n.id.as_str()).copied().unwrap_or(0));
    }
    rollup_directories(&mut nodes, &mut edges);

    Ok(CodeGraph {
        version: 2,
        root: opts.root.clone(),
        generated_at: now(),
        nodes,
        edges,
    })
}

/// Parent directory of a project-relative path, or `None` at the repo root.
pub(super) fn parent_dir(path: &str) -> Option<String> {
    path.rfind('/').map(|i| path[..i].to_string())
}

#[derive(Default)]
struct EdgeSet {
    map: HashMap<(String, String, String), (f32, Option<String>)>,
}

impl EdgeSet {
    fn add(&mut self, source: String, target: String, kind: &str, conf: f32, external: Option<String>) {
        let key = (source, target, kind.to_string());
        let entry = self.map.entry(key).or_insert((0.0, None));
        if conf >= entry.0 {
            entry.0 = conf;
        }
        if entry.1.is_none() {
            entry.1 = external;
        }
    }

    fn into_vec(self) -> Vec<Edge> {
        self.map
            .into_iter()
            .map(|((source, target, kind), (confidence, external))| Edge {
                source,
                target,
                kind,
                confidence,
                external,
            })
            .collect()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Compile a `--path` glob relative to `root` (used by the caller).
pub fn compile_glob(pattern: &str, _root: &Path) -> Result<GlobMatcher> {
    Ok(globset::Glob::new(pattern)?.compile_matcher())
}
