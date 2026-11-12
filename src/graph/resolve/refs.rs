//! Layered resolution of a reference (call / use of a name) to the symbol it
//! points at, with a confidence score.
//!
//! The layers, tried most-certain first:
//!
//! * **L0** — ground truth from a real language server, recorded by
//!   `sync --precise`. It both adds edges the heuristics cannot see and
//!   *removes* ones they would invent, because it knows when a reference
//!   points outside the project.
//! * **L1** — same-file scope resolution, already computed at sync time and
//!   carried on the [`RefRow`] (`local_only` / `resolved_symbol_id`).
//! * **L2** — precise import binding (named imports and `ns.foo()` namespaces).
//! * **L3** — receiver-type resolution: `self`, `Foo::bar`, a declared parameter
//!   type, or a `self.field.method()` chain walked through the type-composition
//!   data.
//! * **L4** — scored disambiguation across same-named definitions; emits only a
//!   clear winner.

use std::collections::{HashMap, HashSet};

use crate::cache::models::{FileRow, RefRow, SymbolRow};

/// What a locally-visible name binds to across files.
#[derive(Debug, Clone)]
pub(super) enum Target {
    Symbols(Vec<i64>),
    Module(i64),
}

/// Read-only indexes the resolver needs, assembled once in [`super::build`].
pub(super) type LocalTypes = HashMap<(i64, String), Vec<(i64, i64, Option<i64>, String)>>;

pub(super) struct ResolveCtx<'a> {
    pub file_by_id: &'a HashMap<i64, &'a FileRow>,
    pub sym_by_id: &'a HashMap<i64, &'a SymbolRow>,
    pub syms_by_file: &'a HashMap<i64, Vec<&'a SymbolRow>>,
    pub defs_by_name: &'a HashMap<&'a str, Vec<&'a SymbolRow>>,
    /// (file id, locally-visible name) -> what it binds to.
    pub binding: &'a HashMap<(i64, String), Target>,
    /// type simple name -> { method name -> symbol ids, one per same-named type }.
    pub type_methods: &'a HashMap<String, HashMap<String, Vec<i64>>>,
    /// type simple name -> { field name -> field type simple name }.
    pub type_fields: &'a HashMap<String, HashMap<String, String>>,
    /// (enclosing symbol id, param/local name) -> declared type simple name.
    pub local_types: &'a LocalTypes,
    /// file id -> file ids it imports (module reachability for L4).
    pub imports_files: &'a HashMap<i64, HashSet<i64>>,
}

/// Evidence attached to the single selected edge. Alternatives are not graph edges.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Resolution {
    #[serde(skip)]
    pub target: i64,
    pub evidence: String,
    pub reason: String,
    pub confidence: f32,
    pub ambiguous: bool,
    pub alternatives: Vec<Alternative>,
    pub omitted_alternatives: usize,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Alternative {
    pub id: String,
    pub qualified_name: String,
    pub path: String,
}
fn facts_kind(s: &SymbolRow) -> crate::analysis::semantic::SymbolKind {
    use crate::analysis::semantic::SymbolKind;
    if s.semantic.kind == SymbolKind::Unknown {
        SymbolKind::from_legacy(&s.kind)
    } else {
        s.semantic.kind
    }
}
fn qualified(s: &SymbolRow) -> &str {
    if s.semantic.qualified_name.is_empty() {
        &s.name
    } else {
        &s.semantic.qualified_name
    }
}
fn same_type(s: &SymbolRow, ty: &str) -> bool {
    let owner = s
        .semantic
        .owner
        .as_deref()
        .or(s.type_name.as_deref())
        .unwrap_or("");
    owner == ty
        || base_type(owner) == base_type(ty)
        || (!ty.contains("::") && owner.rsplit("::").next() == Some(base_type(ty)))
}
fn visible(s: &SymbolRow, rf: &RefRow, importer: &FileRow, ctx: &ResolveCtx) -> bool {
    use crate::analysis::semantic::Visibility;
    match s.semantic.visibility {
        Visibility::Protected if importer.language == "cpp" => {
            let Some(owner) = rf
                .from_symbol_id
                .and_then(|id| ctx.sym_by_id.get(&id))
                .and_then(|from| from.semantic.owner.as_deref())
            else {
                return false;
            };
            s.semantic.owner.as_deref() == Some(owner) || inherited_owner(s, owner, importer, ctx)
        }
        Visibility::Private => {
            if importer.language == "cpp"
                && s.semantic.owner.is_some()
                && rf
                    .from_symbol_id
                    .and_then(|id| ctx.sym_by_id.get(&id))
                    .is_some_and(|from| from.semantic.owner == s.semantic.owner)
            {
                return true;
            }
            if s.file_id != importer.id {
                return false;
            }
            // Rust private items are visible within their module and children.
            if importer.language == "rust" {
                return true;
            }
            match (
                &s.semantic.owner,
                rf.from_symbol_id.and_then(|id| ctx.sym_by_id.get(&id)),
            ) {
                (Some(owner), Some(from)) => qualified(from).starts_with(&format!("{owner}::")),
                _ => true,
            }
        }
        Visibility::Package => {
            let package = ctx
                .syms_by_file
                .get(&importer.id)
                .and_then(|ss| ss.iter().find_map(|s| s.semantic.package.as_ref()));
            s.semantic.package.as_ref() == package
        }
        _ => true,
    }
}

pub(super) fn resolve_ref(rf: &RefRow, importer: &FileRow, ctx: &ResolveCtx) -> Option<Resolution> {
    let exact = |target, confidence, evidence: &str, reason: &str| Resolution {
        target,
        confidence,
        evidence: evidence.into(),
        reason: reason.into(),
        ambiguous: false,
        alternatives: Vec::new(),
        omitted_alternatives: 0,
    };
    match rf.precise_status.as_deref() {
        Some("hit")
            if rf
                .precise_symbol_id
                .is_some_and(|id| ctx.sym_by_id.contains_key(&id)) =>
        {
            return Some(exact(
                rf.precise_symbol_id?,
                rf.precise_confidence.unwrap_or(1.0) as f32,
                "lsp",
                "language-server definition",
            ));
        }
        Some("external" | "nonode" | "ambiguous") => return None,
        _ => {}
    }
    if rf.local_only {
        return None;
    }
    let rk = rf.receiver_kind.as_deref().unwrap_or("none");
    let inferred = infer_receiver_type(rf, ctx);
    let mut all = ctx
        .defs_by_name
        .get(rf.name.as_str())
        .cloned()
        .unwrap_or_default();
    if let Some(Target::Symbols(ids)) = ctx.binding.get(&(rf.file_id, rf.name.clone())) {
        for id in ids {
            if let Some(&s) = ctx.sym_by_id.get(id)
                && !all.iter().any(|a| a.id == s.id)
            {
                all.push(s);
            }
        }
    }
    let source = rf
        .from_symbol_id
        .and_then(|id| ctx.sym_by_id.get(&id))
        .copied();
    let reachable = ctx.imports_files.get(&rf.file_id);
    let binding = ctx.binding.get(&(rf.file_id, rf.name.clone()));
    let mut ranked: Vec<(i32, &SymbolRow, &'static str)> = Vec::new();
    for &s in &all {
        if !ctx
            .file_by_id
            .get(&s.file_id)
            .is_some_and(|f| languages_compatible(&f.language, &importer.language))
            || !visible(s, rf, importer, ctx)
        {
            continue;
        }
        let kind = facts_kind(s);
        if matches!(importer.language.as_str(), "c" | "cpp") {
            let internal = s
                .semantic
                .language_details
                .get("internal_linkage")
                .and_then(|v| v.as_bool())
                == Some(true);
            if internal && s.file_id != rf.file_id {
                continue;
            }
            if s.file_id == rf.file_id
                && s.start_byte
                    .zip(rf.name_start_byte)
                    .is_some_and(|(decl, at)| decl > at)
                && !(importer.language == "cpp"
                    && source.is_some_and(|from| {
                        from.semantic.owner.is_some() && from.semantic.owner == s.semantic.owner
                    }))
            {
                continue;
            }
            if rf.ref_kind == "tag"
                && !matches!(
                    kind,
                    crate::analysis::semantic::SymbolKind::Struct
                        | crate::analysis::semantic::SymbolKind::Union
                        | crate::analysis::semantic::SymbolKind::Enum
                )
            {
                continue;
            }
            if importer.language == "c"
                && rf.ref_kind == "type"
                && matches!(
                    kind,
                    crate::analysis::semantic::SymbolKind::Struct
                        | crate::analysis::semantic::SymbolKind::Union
                        | crate::analysis::semantic::SymbolKind::Enum
                )
            {
                continue;
            }
            if s.file_id != rf.file_id && !reachable.is_some_and(|fs| fs.contains(&s.file_id)) {
                continue;
            }
        }
        if rf.ref_kind == "type" && !kind.is_type() {
            continue;
        }
        if rf.ref_kind == "call" && !kind.callable() && !kind.is_type() && s.kind != "variable" {
            continue;
        }
        if let Some(a) = rf.arg_count {
            let explicit_receiver = rk == "path"
                && importer.language == "rust"
                && s.semantic
                    .parameters
                    .as_ref()
                    .is_some_and(|ps| ps.iter().any(|p| p.receiver));
            if kind.callable() && !s.semantic.accepts_arity(a - i64::from(explicit_receiver)) {
                continue;
            }
        }
        if importer.language == "cpp"
            && rk == "path"
            && let Some(receiver) = &rf.receiver
            && s.semantic.owner.as_deref() != Some(receiver.as_str())
            && !s
                .semantic
                .owner
                .as_deref()
                .is_some_and(|o| o.ends_with(&format!("::{receiver}")))
        {
            continue;
        }
        let mut score = if kind == crate::analysis::semantic::SymbolKind::Constructor {
            5
        } else {
            0
        };
        let mut evidence = "name_heuristic";
        if importer.language == "cpp" && rk == "path" && rf.receiver.is_some() {
            score += 80;
            evidence = "qualified_name";
        }
        if Some(s.id) == rf.resolved_symbol_id {
            score += 100;
            evidence = "scope";
        }
        if let Some(Target::Symbols(ids)) = binding
            && ids.contains(&s.id)
        {
            score += 60;
            evidence = "import";
        }
        if let Some((ty, _)) = &inferred {
            if same_type(s, ty) {
                score += 80;
                evidence = "receiver";
            } else if inherited_owner(s, ty, importer, ctx) {
                score += 65;
                evidence = "inheritance";
            } else if matches!(rk, "value" | "self") && s.semantic.extension_receiver.is_none() {
                continue;
            }
        }
        if matches!(rk, "path" | "value" | "self")
            && let Some(recv) = &rf.receiver
        {
            let key = recv.split(['.', ':']).next().unwrap_or(recv);
            if let Some(Target::Module(fid)) = ctx.binding.get(&(rf.file_id, key.to_string()))
                && *fid == s.file_id
            {
                score += 60;
                evidence = "import";
            }
        }
        if let Some(from) = source {
            if from.semantic.owner.is_some() && from.semantic.owner == s.semantic.owner {
                score += 35;
                if evidence == "name_heuristic" {
                    evidence = "owner";
                }
            }
            if from.semantic.package.is_some() && from.semantic.package == s.semantic.package {
                score += 10;
            }
        }
        if s.file_id == importer.id {
            score += 15;
        }
        if reachable.is_some_and(|r| r.contains(&s.file_id)) {
            score += 10;
        }
        if rf.arg_count.zip(s.param_count).is_some_and(|(a, p)| a == p) {
            score += 3;
        }
        if let (Some(args), Some(params)) = (&rf.argument_types, &s.semantic.parameters) {
            let params: Vec<_> = params.iter().filter(|p| !p.receiver).collect();
            for (arg, param) in args.iter().zip(params) {
                if let (Some(arg), Some(declared)) = (arg, &param.declared_type)
                    && arg.trim() == declared.trim()
                {
                    score += 8;
                }
            }
        }
        if s.is_exported {
            score += 1;
        }
        // Keep the existing foreign/stdlib safeguards. Name-only matches are
        // insufficient evidence for ubiquitous library identifiers.
        if score < 35
            && ((rf.ref_kind == "type" && STD_TYPE_NAMES.contains(&rf.name.as_str()))
                || (rf.ref_kind == "call"
                    && STD_METHOD_NAMES.contains(&rf.name.as_str())
                    && !reachable.is_some_and(|r| r.contains(&s.file_id))))
        {
            continue;
        }
        if matches!(importer.language.as_str(), "rust" | "go")
            && rk == "path"
            && names_foreign_type(rf, ctx)
            && score < 60
        {
            continue;
        }
        // Additional receiver/owner hints must not downgrade a proven lexical binding.
        if Some(s.id) == rf.resolved_symbol_id {
            evidence = "scope";
        }
        ranked.push((score, s, evidence));
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| qualified(a.1).cmp(qualified(b.1)))
            .then_with(|| {
                ctx.file_by_id[&a.1.file_id]
                    .path
                    .cmp(&ctx.file_by_id[&b.1.file_id].path)
            })
            .then_with(|| a.1.start_byte.cmp(&b.1.start_byte))
            .then_with(|| a.1.end_byte.cmp(&b.1.end_byte))
    });
    let &(score, best, evidence) = ranked.first()?;
    let unknown_receiver =
        matches!(rk, "value" | "self") && inferred.is_none() && evidence != "import";
    let dynamic_dispatch = inferred.as_ref().is_some_and(|(ty, _)| {
        ctx.syms_by_file.values().flatten().any(|s| {
            matches!(
                facts_kind(s),
                crate::analysis::semantic::SymbolKind::Interface
                    | crate::analysis::semantic::SymbolKind::Trait
            ) && (qualified(s) == ty || s.name == *ty)
        })
    });
    let ambiguous = unknown_receiver
        || dynamic_dispatch
        || ranked.get(1).is_some_and(|r| score - r.0 < 5)
        || matches!(importer.language.as_str(), "c" | "cpp")
            && (ranked[0]
                .1
                .semantic
                .language_details
                .get("conditional")
                .and_then(|v| v.as_bool())
                == Some(true)
                || (ranked[0].1.semantic.modifiers.is_virtual == Some(true)
                    || ranked[0]
                        .1
                        .semantic
                        .language_details
                        .get("override")
                        .and_then(|v| v.as_bool())
                        == Some(true))
                || !ranked[0].1.semantic.type_parameters.is_empty());
    let alternatives: Vec<_> = if ambiguous {
        ranked
            .iter()
            .skip(1)
            .filter(|r| unknown_receiver || dynamic_dispatch || score - r.0 < 5)
            .map(|(_, s, _)| {
                let path = ctx.file_by_id[&s.file_id].path.clone();
                Alternative {
                    id: crate::graph::declaration_id(
                        &path,
                        &s.name,
                        s.start_byte.unwrap_or(0),
                        s.end_byte.unwrap_or(0),
                        &s.kind,
                    ),
                    qualified_name: qualified(s).into(),
                    path,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let omitted_alternatives = alternatives.len().saturating_sub(3);
    let confidence = if ambiguous {
        0.35
    } else {
        match evidence {
            "scope" => 0.95,
            "import" => 0.90,
            "receiver" => 0.85,
            "inheritance" | "owner" => 0.80,
            _ => 0.60,
        }
    };
    Some(Resolution {
        target: best.id,
        confidence,
        evidence: evidence.into(),
        reason: if unknown_receiver {
            "receiver type is unknown; primary target is a guess".into()
        } else if dynamic_dispatch {
            "receiver uses interface/trait dispatch; runtime implementation is unknown".into()
        } else if ambiguous {
            "multiple compatible candidates; deterministic primary guess".into()
        } else {
            format!("best compatible candidate by {evidence}")
        },
        ambiguous,
        omitted_alternatives,
        alternatives: alternatives.into_iter().take(3).collect(),
    })
}

fn inherited_owner(s: &SymbolRow, ty: &str, importer: &FileRow, ctx: &ResolveCtx) -> bool {
    let mut pending = vec![ty.to_string()];
    let mut seen = HashSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        for types in ctx.syms_by_file.values() {
            for t in types.iter().filter(|t| {
                facts_kind(t).is_type()
                    && (qualified(t) == current
                        || (t.name == current
                            && (t.file_id == importer.id
                                || ctx
                                    .imports_files
                                    .get(&importer.id)
                                    .is_some_and(|files| files.contains(&t.file_id))
                                || t.semantic.package.as_ref().is_some_and(|pkg| {
                                    ctx.syms_by_file.get(&importer.id).is_some_and(|ss| {
                                        ss.iter().any(|s| s.semantic.package.as_ref() == Some(pkg))
                                    })
                                }))))
            }) {
                for base in &t.semantic.relations {
                    if same_type(s, &base.target) {
                        return true;
                    }
                    pending.push(base.target.clone());
                }
            }
        }
    }
    false
}

/// The bare type name inside a written type: `&mut Gauge<'a, T>` and
/// `impl<T> Trait for Gauge<T>` both compare as `Gauge`.
pub(super) fn base_type(written: &str) -> &str {
    let written = written.rsplit(" for ").next().unwrap_or(written);
    let written = written.split('<').next().unwrap_or(written);
    written
        .trim_start_matches(['&', '*'])
        .split_whitespace()
        .rfind(|w| !matches!(*w, "mut" | "const" | "dyn" | "impl") && !w.starts_with('\''))
        .unwrap_or("")
}

/// `Type::method()` where `Type` looks like a type name (`SystemTime`,
/// `Default`) that no project file defines, so it cannot be ours.
fn names_foreign_type(rf: &RefRow, ctx: &ResolveCtx) -> bool {
    let Some(recv) = rf.receiver.as_deref() else {
        return false;
    };
    let head = recv.rsplit("::").next().unwrap_or(recv);
    head.chars().next().is_some_and(char::is_uppercase)
        && head != "Self"
        && !is_type_name(ctx, head)
}

/// Type names the standard library (and the common ecosystem crates) already
/// own. A same-named project type is only credible when an import connects the
/// two files.
const STD_TYPE_NAMES: &[&str] = &[
    "Command",
    "Error",
    "Result",
    "Option",
    "Path",
    "PathBuf",
    "Duration",
    "File",
    "Entry",
    "Builder",
    "Handle",
    "Sender",
    "Receiver",
    "Config",
    "Context",
    "State",
    "Instant",
    "Range",
    "Output",
    "Child",
    "Args",
    "Event",
    "Message",
    "Request",
    "Response",
    "Value",
    "Node",
    "Parser",
    "Formatter",
    "Writer",
    "Reader",
    "Iter",
    "Item",
    "Key",
    "Id",
    "Name",
    "Type",
];

/// Method names the standard library (and every collection type) already owns.
/// A same-named project method is only credible when the file imports it.
const STD_METHOD_NAMES: &[&str] = &[
    "insert",
    "push",
    "pop",
    "get",
    "remove",
    "contains",
    "extend",
    "default",
    "new",
    "len",
    "iter",
    "next",
    "clone",
    "into",
    "from",
    "map",
    "join",
    "trim",
    "split",
    "parse",
    "read",
    "write",
    "open",
    "close",
    "add",
    "set",
    "clear",
    "first",
    "last",
    "find",
    "filter",
    "as_str",
    "to_string",
    "is_empty",
    "as_ref",
    "as_mut",
    "as_bytes",
    "as_slice",
    "fmt",
    "unwrap",
    "expect",
    "ok",
    "err",
    "to_owned",
    "to_vec",
    "flatten",
    "zip",
    "enumerate",
    "collect",
    "take",
    "skip",
    "any",
    "all",
    "count",
    "sum",
    "min",
    "max",
    "lines",
    "chars",
    "bytes",
    "display",
    "status",
    "execute",
    "run",
    "send",
    "recv",
];

/// Best-effort simple type name of the reference's receiver, with a confidence.
fn infer_receiver_type(rf: &RefRow, ctx: &ResolveCtx) -> Option<(String, f32)> {
    let recv = rf.receiver.as_deref()?;
    match rf.receiver_kind.as_deref().unwrap_or("none") {
        "self" => {
            let from = rf.from_symbol_id?;
            let ty = method_owner_type(ctx, from)?;
            Some((ty, 0.9))
        }
        "path" => {
            // `Foo::bar` / `a::Foo::bar` — the receiver is a type path.
            let head = recv.rsplit("::").next().unwrap_or(recv);
            if head == "Self" {
                return rf
                    .from_symbol_id
                    .and_then(|id| method_owner_type(ctx, id))
                    .map(|ty| (ty, 0.85));
            }
            if ctx.type_methods.contains_key(head) || is_type_name(ctx, head) {
                Some((head.to_string(), 0.85))
            } else {
                None
            }
        }
        "value" => resolve_value_receiver(rf, ctx),
        _ => None,
    }
}

/// `self.pool` / `a.b.c` — walk the field-type chain.
fn resolve_value_receiver(rf: &RefRow, ctx: &ResolveCtx) -> Option<(String, f32)> {
    let recv = rf.receiver.as_deref()?;
    let from = rf.from_symbol_id;
    let mut segs = recv.split('.');
    let first = segs.next()?;

    let mut cur = if matches!(first, "self" | "this") {
        method_owner_type(ctx, from?)?
    } else if let Some(ty) = from
        .and_then(|f| ctx.local_types.get(&(f, first.to_string())))
        .and_then(|types| {
            types
                .iter()
                .filter(|(lo, hi, decl, _)| {
                    rf.name_start_byte
                        .is_none_or(|at| at >= *lo && at < *hi && decl.is_none_or(|d| d <= at))
                })
                .min_by_key(|(lo, hi, _, _)| hi - lo)
                .map(|(_, _, _, ty)| ty)
        })
    {
        ty.clone()
    } else if is_type_name(ctx, first) {
        first.to_string()
    } else {
        return None;
    };

    let mut conf = 0.85;
    for seg in segs {
        let next = ctx.type_fields.get(&cur).and_then(|m| m.get(seg))?;
        cur = next.clone();
        conf = 0.8;
    }
    Some((cur, conf))
}

/// The type that owns `symbol_id` if it is a method.
fn method_owner_type(ctx: &ResolveCtx, symbol_id: i64) -> Option<String> {
    let s = ctx.sym_by_id.get(&symbol_id)?;
    if let Some(t) = &s.semantic.owner {
        return Some(t.clone());
    }
    if let Some(t) = &s.type_name {
        return Some(t.clone());
    }
    // Fall back to the parent symbol's name (a class / struct / impl).
    let p = s.parent_symbol_id.and_then(|pid| ctx.sym_by_id.get(&pid))?;
    let n = p.name.rsplit(" for ").next().unwrap_or(&p.name).trim();
    (!n.is_empty()).then(|| n.to_string())
}

fn is_type_name(ctx: &ResolveCtx, name: &str) -> bool {
    ctx.defs_by_name
        .get(name)
        .is_some_and(|list| list.iter().any(|s| is_type_kind(&s.kind)))
}

/// Symbol kinds that a name in type position can legitimately reach.
fn is_type_kind(kind: &str) -> bool {
    crate::analysis::semantic::SymbolKind::from_legacy(kind).is_type()
}

pub(super) fn languages_compatible(a: &str, b: &str) -> bool {
    if matches!(a, "c" | "cpp") && matches!(b, "c" | "cpp") {
        return true;
    }
    if a == b {
        return true;
    }
    let is_js_family = |lang: &str| matches!(lang, "javascript" | "typescript" | "vue" | "svelte");
    if is_js_family(a) && is_js_family(b) {
        return true;
    }
    // Java and Kotlin share the JVM and routinely call into each other.
    let is_jvm_family = |lang: &str| matches!(lang, "java" | "kotlin");
    is_jvm_family(a) && is_jvm_family(b)
}

#[cfg(test)]
mod tests {
    use super::base_type;

    #[test]
    fn base_type_strips_references_generics_and_impl_headers() {
        assert_eq!(base_type("Gauge"), "Gauge");
        assert_eq!(base_type("&Gauge"), "Gauge");
        assert_eq!(base_type("&mut Gauge"), "Gauge");
        assert_eq!(base_type("&'a mut Gauge<'a, T>"), "Gauge");
        assert_eq!(base_type("CustomBuffer<String>"), "CustomBuffer");
        assert_eq!(base_type("Display for Gauge<T>"), "Gauge");
        assert_eq!(base_type("*const Gauge"), "Gauge");
        assert_eq!(base_type(""), "");
    }
}
