//! Turn cached files/symbols/imports/refs into a resolved [`CodeGraph`].

mod imports;
mod postprocess;
mod refs;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use globset::GlobMatcher;

use crate::cache::models::{FileRow, ImportRow, SymbolRow};
use crate::cache::CacheDb;
use crate::graph::{external_id, file_id, symbol_id, CodeGraph, Edge, Node};

use imports::{external_root, resolve_import};
use postprocess::{
    apply_focus, collapse_to_files, degree_map, enforce_max_nodes, prune_unreferenced_externals,
    rollup_directories,
};
use refs::{resolve_ref, ResolveCtx, Target};

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
    let scopes = db.all_scopes()?;
    let bindings = db.all_bindings()?;

    let file_by_id: HashMap<i64, &FileRow> = files.iter().map(|f| (f.id, f)).collect();
    let path_set: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let frow_by_path: HashMap<&str, &FileRow> =
        files.iter().map(|f| (f.path.as_str(), f)).collect();

    let sym_by_id: HashMap<i64, &SymbolRow> = symbols.iter().map(|s| (s.id, s)).collect();
    let mut syms_by_file: HashMap<i64, Vec<&SymbolRow>> = HashMap::new();
    for s in &symbols {
        syms_by_file.entry(s.file_id).or_default().push(s);
    }
    let import_by_id: HashMap<i64, &ImportRow> = imports.iter().map(|i| (i.id, i)).collect();
    let scope_owner: HashMap<i64, Option<i64>> =
        scopes.iter().map(|s| (s.id, s.owner_symbol_id)).collect();

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

    // ---- imports: file->file edges, externals, module reachability -------
    // file id -> file ids it imports (used by L4 disambiguation scoring).
    let mut imports_files: HashMap<i64, HashSet<i64>> = HashMap::new();

    for im in &imports {
        let Some(importer) = file_by_id.get(&im.file_id) else {
            continue;
        };
        let resolved = resolve_import(&importer.language, &importer.path, im, &path_set);

        if let Some(target_path) = resolved.as_deref() {
            if let Some(tf) = frow_by_path.get(target_path) {
                if tf.id != im.file_id {
                    imports_files.entry(im.file_id).or_default().insert(tf.id);
                }
            }
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

    // Locally-visible name -> cross-file target, from the `bindings` table.
    let mut binding: HashMap<(i64, String), Target> = HashMap::new();
    for b in &bindings {
        let Some(imp) = b.import_id.and_then(|i| import_by_id.get(&i)) else {
            continue;
        };
        let Some(importer) = file_by_id.get(&b.file_id) else {
            continue;
        };
        let Some(tpath) = resolve_import(&importer.language, &importer.path, imp, &path_set) else {
            continue;
        };
        let Some(tf) = frow_by_path.get(tpath.as_str()) else {
            continue;
        };
        match b.binding_kind.as_str() {
            "import" => {
                let want = imp.imported_name.as_deref().unwrap_or(b.name.as_str());
                let sym = syms_by_file.get(&tf.id).and_then(|list| {
                    list.iter()
                        .find(|s| s.name == want && s.is_exported)
                        .or_else(|| list.iter().find(|s| s.name == want))
                });
                match sym {
                    Some(s) => {
                        binding.insert((b.file_id, b.name.clone()), Target::Symbol(s.id));
                    }
                    // e.g. Rust `use crate::scheduler;` — a module, not a symbol.
                    None => {
                        binding.insert((b.file_id, b.name.clone()), Target::Module(tf.id));
                    }
                }
            }
            "namespace" => {
                binding.insert((b.file_id, b.name.clone()), Target::Module(tf.id));
            }
            _ => {}
        }
    }

    // type simple name -> { method name -> symbol id }
    let mut type_methods: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for s in &symbols {
        if s.kind == "method" {
            if let Some(t) = &s.type_name {
                type_methods
                    .entry(t.clone())
                    .or_default()
                    .entry(s.name.clone())
                    .or_insert(s.id);
            }
        }
    }

    // type simple name -> { field name -> field type }, and
    // (enclosing symbol id, param/local name) -> declared type.
    let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut local_types: HashMap<(i64, String), String> = HashMap::new();
    for b in &bindings {
        let Some(ty) = b.type_expr.clone() else {
            continue;
        };
        let owner = scope_owner.get(&b.scope_id).copied().flatten();
        match b.binding_kind.as_str() {
            "field" => {
                if let Some(owner_sym) = owner.and_then(|id| sym_by_id.get(&id)) {
                    type_fields
                        .entry(owner_sym.name.clone())
                        .or_default()
                        .insert(b.name.clone(), ty);
                }
            }
            "param" | "local" => {
                if let Some(oid) = owner {
                    local_types.insert((oid, b.name.clone()), ty);
                }
            }
            _ => {}
        }
    }

    let ctx = ResolveCtx {
        file_by_id: &file_by_id,
        sym_by_id: &sym_by_id,
        syms_by_file: &syms_by_file,
        defs_by_name: &defs_by_name,
        binding: &binding,
        type_methods: &type_methods,
        type_fields: &type_fields,
        local_types: &local_types,
        imports_files: &imports_files,
    };

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

        let Some((target, confidence)) = resolve_ref(rf, importer, &ctx) else {
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
