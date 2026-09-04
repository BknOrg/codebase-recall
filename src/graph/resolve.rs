//! Turn cached files/symbols/imports/refs into a resolved [`CodeGraph`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::Result;
use globset::GlobMatcher;

use crate::cache::models::{FileRow, ImportRow, RefRow, SymbolRow};
use crate::cache::CacheDb;
use crate::graph::{external_id, file_id, symbol_id, CodeGraph, Edge, Node};

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
            language: Some(f.language.clone()),
            exported: true,
            lines: None,
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
                language: Some(f.language.clone()),
                exported: s.is_exported,
                lines: Some([start, s.end_line.unwrap_or(start)]),
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
                    language: None,
                    exported: false,
                    lines: None,
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

    Ok(CodeGraph {
        version: 1,
        root: opts.root.clone(),
        generated_at: now(),
        nodes,
        edges,
    })
}

// ---------------------------------------------------------------------------
// Reference resolution
// ---------------------------------------------------------------------------

fn resolve_ref(
    rf: &RefRow,
    importer: &FileRow,
    syms_by_file: &HashMap<i64, Vec<&SymbolRow>>,
    binding: &HashMap<(i64, String), i64>,
    defs_by_name: &HashMap<&str, Vec<&SymbolRow>>,
    file_by_id: &HashMap<i64, &FileRow>,
) -> Option<(i64, f32)> {
    // 1. same-file definition (exact scope match wins outright).
    if let Some(list) = syms_by_file.get(&rf.file_id) {
        if let Some(s) = list.iter().find(|s| s.name == rf.name) {
            return Some((s.id, 1.0));
        }
    }
    // 2. imported binding.
    if let Some(&sid) = binding.get(&(rf.file_id, rf.name.clone())) {
        return Some((sid, 1.0));
    }

    // Only consider definitions in files of the same language.
    let all = defs_by_name.get(rf.name.as_str())?;
    let list: Vec<&&SymbolRow> = all
        .iter()
        .filter(|s| {
            file_by_id
                .get(&s.file_id)
                .is_some_and(|f| f.language == importer.language)
        })
        .collect();
    if list.is_empty() {
        return None;
    }

    let unique_method = || {
        let m: Vec<_> = list.iter().filter(|s| s.kind == "method").collect();
        (m.len() == 1).then(|| m[0].id)
    };
    let unique_exported = || {
        let e: Vec<_> = list.iter().filter(|s| s.is_exported).collect();
        (e.len() == 1).then(|| e[0].id)
    };
    let unique_any = || (list.len() == 1).then(|| list[0].id);

    match classify_receiver(rf.receiver.as_deref()) {
        // `expr.method()` — value receiver: only an unambiguous method.
        Receiver::Value => unique_method().map(|id| (id, 0.45)),
        // `foo::bar()` / `Foo::bar()` — could be an associated fn or a
        // module-qualified free function.
        Receiver::Path => unique_method()
            .map(|id| (id, 0.55))
            .or_else(|| unique_exported().map(|id| (id, 0.6)))
            .or_else(|| unique_any().map(|id| (id, 0.45))),
        // Bare `name(...)` — unique project-wide definition.
        Receiver::None => unique_exported()
            .map(|id| (id, 0.7))
            .or_else(|| unique_any().map(|id| (id, 0.5))),
    }
}

enum Receiver {
    None,
    /// `foo::bar()` / `Foo::Bar::baz()` — a plain path expression.
    Path,
    /// `expr.method()` — a value/field/self receiver.
    Value,
}

fn classify_receiver(recv: Option<&str>) -> Receiver {
    let Some(r) = recv else {
        return Receiver::None;
    };
    if matches!(r, "self" | "Self" | "this") {
        return Receiver::Value;
    }
    let is_path = !r.is_empty()
        && r.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':');
    if is_path {
        Receiver::Path
    } else {
        Receiver::Value
    }
}

// ---------------------------------------------------------------------------
// Import resolution
// ---------------------------------------------------------------------------

fn resolve_import(
    lang_group: &str,
    importer_rel: &str,
    im: &ImportRow,
    path_set: &HashSet<String>,
) -> Option<String> {
    match lang_group {
        "rust" => resolve_rust_import(importer_rel, &im.raw_specifier, path_set),
        "javascript" | "typescript" => {
            resolve_js_import(importer_rel, &im.raw_specifier, path_set)
        }
        "python" => resolve_python_import(importer_rel, &im.raw_specifier, path_set),
        _ => None,
    }
}

fn dir_components(rel: &str) -> Vec<String> {
    let mut c: Vec<String> = rel.split('/').map(str::to_string).collect();
    c.pop(); // drop file name
    c
}

fn first_existing(path_set: &HashSet<String>, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .find(|c| path_set.contains(*c))
        .cloned()
}

fn resolve_rust_import(
    importer_rel: &str,
    spec: &str,
    path_set: &HashSet<String>,
) -> Option<String> {
    let segs: Vec<&str> = spec.split("::").filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }

    let mut base: Vec<String>;
    let rest: &[&str];
    match segs[0] {
        "crate" => {
            base = vec!["src".to_string()];
            rest = &segs[1..];
        }
        "self" => {
            base = dir_components(importer_rel);
            rest = &segs[1..];
        }
        "super" => {
            base = dir_components(importer_rel);
            let mut i = 0;
            while i < segs.len() && segs[i] == "super" {
                base.pop();
                i += 1;
            }
            rest = &segs[i..];
        }
        _ => {
            // Possibly a local top-level module; otherwise an external crate.
            base = vec!["src".to_string()];
            rest = &segs[..];
        }
    }

    let mut module: Vec<String> = base.drain(..).collect();
    module.extend(rest.iter().map(|s| s.to_string()));

    // Try the full module path, then drop the trailing item name.
    for cut in [0usize, 1] {
        if module.len() <= cut {
            continue;
        }
        let mp = &module[..module.len() - cut];
        let joined = mp.join("/");
        let cands = [
            format!("{joined}.rs"),
            format!("{joined}/mod.rs"),
            joined.strip_prefix("src/").map(|s| format!("{s}.rs")).unwrap_or_default(),
        ];
        if let Some(hit) = first_existing(path_set, &cands) {
            return Some(hit);
        }
    }
    None
}

fn normalize_join(dir: &[String], spec: &str) -> Vec<String> {
    let mut out: Vec<String> = dir.to_vec();
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p.to_string()),
        }
    }
    out
}

fn resolve_js_import(
    importer_rel: &str,
    spec: &str,
    path_set: &HashSet<String>,
) -> Option<String> {
    if !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
        return None; // bare specifier -> external
    }
    let joined = normalize_join(&dir_components(importer_rel), spec).join("/");
    const EXTS: &[&str] = &["", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".d.ts"];
    const INDEX: &[&str] = &[
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
        "/index.mjs",
    ];
    for e in EXTS {
        let c = format!("{joined}{e}");
        if path_set.contains(&c) {
            return Some(c);
        }
    }
    for i in INDEX {
        let c = format!("{joined}{i}");
        if path_set.contains(&c) {
            return Some(c);
        }
    }
    None
}

fn resolve_python_import(
    importer_rel: &str,
    spec: &str,
    path_set: &HashSet<String>,
) -> Option<String> {
    let dots = spec.chars().take_while(|c| *c == '.').count();
    let tail = &spec[dots..];
    let parts: Vec<String> = if tail.is_empty() {
        Vec::new()
    } else {
        tail.split('.').map(str::to_string).collect()
    };

    let mut bases: Vec<Vec<String>> = Vec::new();
    if dots > 0 {
        let mut b = dir_components(importer_rel);
        for _ in 1..dots {
            b.pop();
        }
        bases.push(b);
    } else {
        bases.push(Vec::new());
        bases.push(vec!["src".to_string()]);
    }

    for base in bases {
        for cut in [0usize, 1] {
            if parts.len() < cut {
                continue;
            }
            let mut mp = base.clone();
            mp.extend_from_slice(&parts[..parts.len() - cut]);
            if mp.is_empty() {
                continue;
            }
            let joined = mp.join("/");
            for c in [format!("{joined}.py"), format!("{joined}/__init__.py")] {
                if path_set.contains(&c) {
                    return Some(c);
                }
            }
        }
    }
    None
}

fn external_root(spec: &str, lang_group: &str) -> String {
    match lang_group {
        "rust" => spec.split("::").next().unwrap_or(spec).to_string(),
        "python" => spec.trim_start_matches('.').split('.').next().unwrap_or(spec).to_string(),
        _ => {
            if let Some(scoped) = spec.strip_prefix('@') {
                let mut it = scoped.split('/');
                let a = it.next().unwrap_or("");
                let b = it.next().unwrap_or("");
                format!("@{a}/{b}")
            } else {
                spec.split('/').next().unwrap_or(spec).to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-processing
// ---------------------------------------------------------------------------

fn collapse_to_files(
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    sym_node: &HashMap<i64, (String, String)>,
) {
    let sym_to_file: HashMap<&str, &str> = sym_node
        .values()
        .map(|(s, f)| (s.as_str(), f.as_str()))
        .collect();
    nodes.retain(|n| n.kind == "file" || n.kind == "external");

    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut out = Vec::new();
    for e in edges.drain(..) {
        if e.kind == "contains" {
            continue;
        }
        let s = sym_to_file.get(e.source.as_str()).map(|s| s.to_string()).unwrap_or(e.source);
        let t = sym_to_file.get(e.target.as_str()).map(|s| s.to_string()).unwrap_or(e.target);
        if s == t {
            continue;
        }
        if seen.insert((s.clone(), t.clone(), e.kind.clone())) {
            out.push(Edge {
                source: s,
                target: t,
                kind: e.kind,
                confidence: e.confidence,
                external: e.external,
            });
        }
    }
    *edges = out;
}

fn apply_focus(nodes: &mut Vec<Node>, edges: &mut Vec<Edge>, focus: &str, depth: u32) {
    let seeds: HashSet<&str> = nodes
        .iter()
        .filter(|n| n.label == focus || n.id.contains(focus))
        .map(|n| n.id.as_str())
        .collect();
    if seeds.is_empty() {
        return;
    }

    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges.iter() {
        adj.entry(&e.source).or_default().push(&e.target);
        adj.entry(&e.target).or_default().push(&e.source);
    }

    let mut keep: HashSet<String> = seeds.iter().map(|s| s.to_string()).collect();
    let mut frontier: VecDeque<(String, u32)> =
        seeds.iter().map(|s| (s.to_string(), 0)).collect();
    while let Some((id, d)) = frontier.pop_front() {
        if d >= depth {
            continue;
        }
        if let Some(neigh) = adj.get(id.as_str()) {
            for n in neigh {
                if keep.insert(n.to_string()) {
                    frontier.push_back((n.to_string(), d + 1));
                }
            }
        }
    }

    nodes.retain(|n| keep.contains(&n.id));
    edges.retain(|e| keep.contains(&e.source) && keep.contains(&e.target));
}

fn prune_unreferenced_externals(nodes: &mut Vec<Node>, edges: &[Edge], include_external: bool) {
    if !include_external {
        nodes.retain(|n| n.kind != "external");
        return;
    }
    let used: HashSet<&str> = edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    nodes.retain(|n| n.kind != "external" || used.contains(n.id.as_str()));
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
