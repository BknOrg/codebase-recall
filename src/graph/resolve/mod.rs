//! Turn cached files/symbols/imports/refs into a resolved [`CodeGraph`].

mod imports;
pub(crate) mod index;
mod postprocess;
pub(crate) mod refs;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use globset::GlobMatcher;

use crate::cache::CacheDb;
use crate::cache::models::{FileRow, ImportRow, SymbolRow};
use crate::graph::{CodeGraph, CommunityInfo, Edge, Node, community, external_id, file_id};

use imports::external_root;
use postprocess::{
    apply_focus, collapse_to_files, degree_map, enforce_max_nodes, prune_unreferenced_externals,
    rollup_directories,
};
use refs::{base_type, resolve_ref};

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
    crate::health::ensure_current(db)?;
    let files = db.all_files()?;
    let symbols = db.all_symbols()?;
    let imports = db.all_imports()?;
    let refs = db.all_refs()?;
    let scopes = db.all_scopes()?;
    let bindings = db.all_bindings()?;

    let index =
        index::ReferenceIndex::new(&db.project, &files, &symbols, &imports, &scopes, &bindings)?;
    let file_by_id = &index.file_by_id;
    let sym_by_id = &index.sym_by_id;
    let defs_by_name = &index.defs_by_name;
    let imports_files = &index.imports_files;
    let resolve_file =
        |_file: &FileRow, im: &ImportRow| index.resolved_imports.get(&im.id).cloned().flatten();
    // Does this file pass the --path glob?
    let keep_file = |path: &str| opts.path_glob.as_ref().is_none_or(|g| g.is_match(path));

    // ---- nodes -------------------------------------------------------------
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_ids: HashSet<String> = HashSet::new();
    // sym db id -> (node id string, owning file node id string)
    let mut sym_node: HashMap<i64, (String, String)> = HashMap::new();

    for f in &files {
        crate::operation::check()?;
        // Config files are indexed for their keys, not their structure; they
        // define no symbols and import nothing, so a node for one would only
        // ever sit unconnected in the graph.
        if !keep_file(&f.path) || crate::analysis::lang::is_config_group(&f.language) {
            continue;
        }
        let id = file_id(&f.path);
        node_ids.insert(id.clone());
        nodes.push(Node {
            semantic: None,
            id,
            kind: "file".into(),
            label: f.path.clone(),
            path: Some(f.path.clone()),
            dir: parent_dir(&f.path),
            language: Some(f.language.clone()),
            exported: true,
            lines: None,
            degree: None,
            community: None,
        });
    }

    let want_symbols = opts.scope != Scope::File;
    for s in &symbols {
        crate::operation::check()?;
        let Some(f) = file_by_id.get(&s.file_id) else {
            continue;
        };
        if !keep_file(&f.path) {
            continue;
        }
        let start = s.start_line.unwrap_or(0);
        let sid = crate::graph::declaration_id(
            &f.path,
            &s.name,
            s.start_byte.unwrap_or(0),
            s.end_byte.unwrap_or(0),
            &s.kind,
        );
        sym_node.insert(s.id, (sid.clone(), file_id(&f.path)));
        if want_symbols {
            node_ids.insert(sid.clone());
            nodes.push(Node {
                semantic: Some(s.semantic.clone()),
                id: sid,
                kind: s.kind.to_string(),
                label: s.name.clone(),
                path: Some(f.path.clone()),
                dir: parent_dir(&f.path),
                language: Some(f.language.clone()),
                exported: s.is_exported,
                lines: Some([start, s.end_line.unwrap_or(start)]),
                degree: None,
                community: None,
            });
        }
    }

    // ---- edge accumulator ------------------------------------------------
    let mut edges: EdgeSet = EdgeSet::default();

    // contains
    if opts.kinds.contains("contains") && want_symbols {
        for s in &symbols {
            crate::operation::check()?;
            let Some((child, file_node)) = sym_node.get(&s.id) else {
                continue;
            };
            let parent = match s.parent_symbol_id.and_then(|pid| sym_node.get(&pid)) {
                Some((p, _)) => p.clone(),
                None => file_node.clone(),
            };
            edges.add(parent, child.clone(), "contains", 1.0, None, s.start_line);
        }
    }

    // ---- imports: file->file edges, externals, module reachability -------
    // file id -> file ids it imports (used by L4 disambiguation scoring).

    for im in &imports {
        crate::operation::check()?;
        let Some(importer) = file_by_id.get(&im.file_id) else {
            continue;
        };
        let resolved = resolve_file(importer, im);

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
                    im.start_line,
                );
            }
        } else if opts.include_external
            && opts.kinds.contains("imports")
            && keep_file(&importer.path)
        {
            let label = external_root(&im.raw_specifier, &importer.language);
            let ext = external_id(&label);
            if node_ids.insert(ext.clone()) {
                nodes.push(Node {
                    semantic: None,
                    id: ext.clone(),
                    kind: "external".into(),
                    label,
                    path: None,
                    dir: None,
                    language: None,
                    exported: false,
                    lines: None,
                    degree: None,
                    community: None,
                });
            }
            edges.add(
                file_id(&importer.path),
                ext,
                "imports",
                1.0,
                Some(im.raw_specifier.clone()),
                im.start_line,
            );
        }
    }

    // A declaration remains its own node. Link it to a unique matching definition
    // only when that definition's translation unit includes the declaration's file.
    if opts.kinds.contains("references") && want_symbols && opts.min_confidence <= 0.35 {
        for definition in symbols.iter().filter(|s| {
            s.semantic.role == crate::analysis::semantic::SymbolRole::Definition
                && s.kind.callable()
                && file_by_id
                    .get(&s.file_id)
                    .is_some_and(|f| matches!(f.language.as_str(), "c" | "cpp"))
        }) {
            let matches: Vec<_> = symbols
                .iter()
                .filter(|s| {
                    s.semantic.role == crate::analysis::semantic::SymbolRole::Declaration
                        && s.kind == definition.kind
                        && s.semantic.qualified_name == definition.semantic.qualified_name
                        && s.semantic.return_type == definition.semantic.return_type
                        && s.semantic.parameters.as_ref().map(|ps| {
                            ps.iter()
                                .map(|p| (&p.declared_type, p.variadic))
                                .collect::<Vec<_>>()
                        }) == definition.semantic.parameters.as_ref().map(|ps| {
                            ps.iter()
                                .map(|p| (&p.declared_type, p.variadic))
                                .collect::<Vec<_>>()
                        })
                        && imports_files
                            .get(&definition.file_id)
                            .is_some_and(|fs| fs.contains(&s.file_id))
                })
                .collect();
            if let [declaration] = matches.as_slice()
                && let (Some((from, _)), Some((to, _))) =
                    (sym_node.get(&declaration.id), sym_node.get(&definition.id))
            {
                edges.add(
                    from.clone(),
                    to.clone(),
                    "references",
                    0.35,
                    None,
                    definition.start_line,
                );
                edges.map.entry((from.clone(),to.clone(),"references".into())).or_default().resolution=Some(refs::Resolution {target:definition.id,evidence:"declaration_definition".into(),reason:"unique compatible definition in a translation unit including the declaration; compiler confirmation required".into(),confidence:0.35,ambiguous:true,alternatives:Vec::new(),omitted_alternatives:0});
            }
        }
    }
    // ---- calls / references -------------------------------------------
    // ---- implements: `impl Trait for Type` -> Type implements Trait -------
    // The analyzer already stores the header verbatim as the `impl` symbol's
    // name ("Display for Gauge<T>"), so both ends are in hand; only the edge
    // was ever missing. A bare `impl Type` block declares no trait and is
    // skipped. Without this a trait has no link to its implementors, so a call
    // through `&dyn Trait` dead-ends at the trait definition.
    if opts.kinds.contains("implements") && want_symbols {
        let pick = |written: &str, want_trait: bool, owner: &SymbolRow| -> Option<&SymbolRow> {
            let defs = defs_by_name.get(base_type(written))?;
            let mut candidates: Vec<_> = defs
                .iter()
                .copied()
                .filter(|s| {
                    if want_trait {
                        matches!(s.kind.as_str(), "trait" | "interface")
                    } else {
                        s.semantic.kind.is_type()
                            || matches!(s.kind.as_str(), "struct" | "enum" | "class" | "type")
                    }
                })
                .collect();
            if candidates.len() > 1 {
                let local: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|s| s.file_id == owner.file_id)
                    .collect();
                if !local.is_empty() {
                    candidates = local;
                }
            }
            if let [only] = candidates.as_slice() {
                Some(*only)
            } else {
                None
            }
        };
        for s in &symbols {
            crate::operation::check()?;
            if s.kind != "impl" {
                continue;
            }
            let Some((trait_written, type_written)) = s.name.split_once(" for ") else {
                continue;
            };
            let (Some(tr), Some(ty)) = (pick(trait_written, true, s), pick(type_written, false, s))
            else {
                continue;
            };
            let (Some((tr_node, _)), Some((ty_node, _))) =
                (sym_node.get(&tr.id), sym_node.get(&ty.id))
            else {
                continue;
            };
            if tr_node != ty_node {
                edges.add(
                    ty_node.clone(),
                    tr_node.clone(),
                    "implements",
                    1.0,
                    None,
                    s.start_line,
                );
            }
        }
    }

    let ctx = index.context();

    let mut resolution_summary = crate::graph::ResolutionSummary::default();
    for rf in &refs {
        crate::operation::check()?;
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

        resolution_summary.eligible += 1;
        let Some(resolution) = resolve_ref(rf, importer, &ctx) else {
            if rf.local_only {
                resolution_summary.local_only += 1;
            } else if matches!(rf.precise_status.as_deref(), Some("external" | "nonode")) {
                resolution_summary.external += 1;
            } else {
                resolution_summary.unresolved += 1;
                if rf.precise_status.as_deref() == Some("ambiguous") {
                    crate::operation::warning(format!(
                        "{}:{}: cached LSP answer has multiple targets; left unresolved",
                        importer.path,
                        rf.start_line.unwrap_or(0)
                    ));
                }
            }
            continue;
        };
        resolution_summary.resolved += 1;
        if resolution.ambiguous {
            resolution_summary.ambiguous += 1;
        }
        let target = resolution.target;
        let confidence = resolution.confidence;
        if confidence < opts.min_confidence {
            resolution_summary.below_min_confidence += 1;
            continue;
        }
        let Some((tgt, tgt_file)) = sym_node.get(&target) else {
            continue;
        };
        let tgt = if want_symbols {
            tgt.clone()
        } else {
            tgt_file.clone()
        };
        if tgt == src {
            continue;
        }
        let key = (src.clone(), tgt.clone(), edge_kind.to_string());
        let member_resolution = resolution.clone();
        let member_target = tgt.clone();
        edges.add(src, tgt, edge_kind, confidence, None, rf.start_line);
        let meta = edges.map.get_mut(&key).unwrap();
        if meta.resolution.as_ref().is_none_or(|r| {
            resolution.confidence > r.confidence
                || (resolution.confidence == r.confidence
                    && meta.line == rf.start_line
                    && resolution.evidence <= r.evidence)
        }) {
            meta.resolution = Some(resolution);
        }
        // Keep the owning type's dependency as well as the precise member site.
        // Otherwise introducing field/variant nodes would make impact stop at
        // the member and lose the pre-existing type dependency.
        if want_symbols
            && rf.ref_kind == "type"
            && let Some(member) = rf.from_symbol_id.and_then(|id| sym_by_id.get(&id))
            && matches!(
                member.semantic.kind,
                crate::analysis::semantic::SymbolKind::Field
                    | crate::analysis::semantic::SymbolKind::Property
                    | crate::analysis::semantic::SymbolKind::EnumMember
            )
            && let Some(owner) = member.parent_symbol_id.and_then(|id| sym_by_id.get(&id))
            && owner.semantic.kind.is_type()
            && let Some((source, _)) = sym_node.get(&owner.id)
            && *source != member_target
        {
            let key = (source.clone(), member_target.clone(), edge_kind.to_string());
            edges.add(
                source.clone(),
                member_target,
                edge_kind,
                confidence,
                None,
                rf.start_line,
            );
            let mut evidence = member_resolution;
            evidence.evidence = "member_type".into();
            evidence.reason = format!(
                "owner depends on the declared type of member {}",
                member.name
            );
            let meta = edges.map.get_mut(&key).unwrap();
            if meta
                .resolution
                .as_ref()
                .is_none_or(|old| evidence.confidence > old.confidence)
            {
                meta.resolution = Some(evidence);
            }
        }
    }

    // Structural relations are opt-in and require an unambiguous type identity.
    for symbol in &symbols {
        if symbol.kind == "impl" {
            continue;
        }
        let Some((source, _)) = sym_node.get(&symbol.id) else {
            continue;
        };
        for relation in &symbol.semantic.relations {
            let kind = match relation.kind {
                crate::analysis::semantic::RelationKind::Implements => "implements",
                crate::analysis::semantic::RelationKind::Embeds => "inherits",
                _ => "inherits",
            };
            let target_name = relation.target.replace('.', "::");
            let target_name = target_name.split('<').next().unwrap_or(&target_name);
            let mut candidates: Vec<_> = symbols
                .iter()
                .filter(|s| {
                    s.semantic.kind.is_type()
                        && (s.semantic.qualified_name == target_name || s.name == target_name)
                        && file_by_id
                            .get(&s.file_id)
                            .zip(file_by_id.get(&symbol.file_id))
                            .is_some_and(|(a, b)| {
                                refs::languages_compatible(&a.language, &b.language)
                            })
                })
                .collect();
            if file_by_id
                .get(&symbol.file_id)
                .is_some_and(|f| f.language == "cpp")
            {
                candidates.retain(|s| {
                    s.file_id == symbol.file_id
                        || imports_files
                            .get(&symbol.file_id)
                            .is_some_and(|fs| fs.contains(&s.file_id))
                });
                if !target_name.contains("::") {
                    let same_namespace: Vec<_> = candidates
                        .iter()
                        .copied()
                        .filter(|s| s.semantic.owner == symbol.semantic.owner)
                        .collect();
                    if !same_namespace.is_empty() {
                        candidates = same_namespace;
                    }
                }
            }
            if candidates.len() > 1 {
                candidates.retain(|s| s.semantic.package == symbol.semantic.package);
            }
            let [base] = candidates.as_slice() else {
                continue;
            };
            let Some((target, _)) = sym_node.get(&base.id) else {
                continue;
            };
            if want_symbols
                && opts.kinds.contains(kind)
                && node_ids.contains(source)
                && node_ids.contains(target)
            {
                edges.add(
                    source.clone(),
                    target.clone(),
                    kind,
                    1.0,
                    None,
                    symbol.start_line,
                );
            }
            if want_symbols && opts.kinds.contains("overrides") {
                for member in symbols
                    .iter()
                    .filter(|m| m.semantic.kind.callable() && m.parent_symbol_id == Some(symbol.id))
                {
                    if member.semantic.modifiers.is_static == Some(true) {
                        continue;
                    }
                    for inherited in symbols.iter().filter(|m| {
                        m.semantic.kind.callable()
                            && m.parent_symbol_id == Some(base.id)
                            && m.name == member.name
                    }) {
                        if file_by_id
                            .get(&member.file_id)
                            .is_some_and(|f| f.language == "cpp")
                            && inherited.semantic.modifiers.is_virtual != Some(true)
                            && member
                                .semantic
                                .language_details
                                .get("override")
                                .and_then(|v| v.as_bool())
                                != Some(true)
                        {
                            continue;
                        }
                        if file_by_id
                            .get(&member.file_id)
                            .is_some_and(|f| f.language == "cpp")
                            && ["const", "volatile", "ref_qualifier"].iter().any(|key| {
                                member.semantic.language_details.get(*key)
                                    != inherited.semantic.language_details.get(*key)
                            })
                        {
                            continue;
                        }
                        let signatures_match =
                            match (&member.semantic.parameters, &inherited.semantic.parameters) {
                                (Some(a), Some(b)) => {
                                    let a: Vec<_> = a.iter().filter(|p| !p.receiver).collect();
                                    let b: Vec<_> = b.iter().filter(|p| !p.receiver).collect();
                                    a.len() == b.len()
                                        && a.iter().zip(b).all(|(x, y)| {
                                            x.declared_type.is_some()
                                                && x.declared_type == y.declared_type
                                        })
                                }
                                _ => false,
                            };
                        if signatures_match
                            && let (Some((a, _)), Some((b, _))) =
                                (sym_node.get(&member.id), sym_node.get(&inherited.id))
                            && node_ids.contains(a)
                            && node_ids.contains(b)
                        {
                            edges.add(
                                a.clone(),
                                b.clone(),
                                "overrides",
                                1.0,
                                None,
                                member.start_line,
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- finalize ------------------------------------------------------
    let mut edges = edges.into_vec();

    // Detect communities on the full edge set, before collapse/focus/node-cap
    // shrink the graph, so filtering a view never changes which subsystem a file is in.
    let communities = community::detect(&nodes, &edges);

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
        n.community = community::file_of(&n.id, n.path.as_deref())
            .and_then(|f| communities.assignment.get(&f).copied());
    }
    rollup_directories(&mut nodes, &mut edges);
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort_by(|a, b| (&a.source, &a.target, &a.kind).cmp(&(&b.source, &b.target, &b.kind)));

    let community_infos: Vec<CommunityInfo> = communities
        .list
        .iter()
        .map(|c| CommunityInfo {
            id: c.id,
            label: c.label.clone(),
            size: c.size,
        })
        .collect();

    Ok(CodeGraph {
        precision: crate::precise::state::metadata(db)?,
        resolution_summary,
        version: 3,
        root: opts.root.clone(),
        generated_at: now(),
        nodes,
        edges,
        communities: community_infos,
    })
}

/// Parent directory of a project-relative path, or `None` at the repo root.
pub(super) fn parent_dir(path: &str) -> Option<String> {
    path.rfind(['/', '\\']).map(|i| path[..i].to_string())
}

type EdgeKey = (String, String, String);

#[derive(Default)]
struct EdgeMeta {
    resolution: Option<refs::Resolution>,
    confidence: f32,
    external: Option<String>,
    line: Option<i64>,
}

#[derive(Default)]
struct EdgeSet {
    map: HashMap<EdgeKey, EdgeMeta>,
}

impl EdgeSet {
    fn add(
        &mut self,
        source: String,
        target: String,
        kind: &str,
        conf: f32,
        external: Option<String>,
        line: Option<i64>,
    ) {
        let key = (source, target, kind.to_string());
        let entry = self.map.entry(key).or_default();
        if conf > entry.confidence {
            entry.confidence = conf;
            entry.line = line;
        } else if conf == entry.confidence {
            entry.line = entry.line.into_iter().chain(line).min();
        }
        if entry.external.is_none() {
            entry.external = external;
        }
        if entry.line.is_none() {
            entry.line = line;
        }
    }

    fn into_vec(self) -> Vec<Edge> {
        let mut edges: Vec<_> = self
            .map
            .into_iter()
            .map(|((source, target, kind), meta)| Edge {
                resolution: meta.resolution,
                source,
                target,
                kind,
                confidence: meta.confidence,
                external: meta.external,
                line: meta.line,
            })
            .collect();
        edges.sort_by(|a, b| (&a.source, &a.target, &a.kind).cmp(&(&b.source, &b.target, &b.kind)));
        edges
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
    let normalized_pattern = pattern.replace('\\', "/");
    let clean_pattern = normalized_pattern
        .strip_prefix("./")
        .unwrap_or(&normalized_pattern);
    Ok(globset::Glob::new(clean_pattern)?.compile_matcher())
}
