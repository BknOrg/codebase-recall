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
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    Symbol(i64),
    Module(i64),
}

/// Read-only indexes the resolver needs, assembled once in [`super::build`].
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
    pub local_types: &'a HashMap<(i64, String), String>,
    /// file id -> file ids it imports (module reachability for L4).
    pub imports_files: &'a HashMap<i64, HashSet<i64>>,
}

pub(super) fn resolve_ref(
    rf: &RefRow,
    importer: &FileRow,
    ctx: &ResolveCtx,
) -> Option<(i64, f32)> {
    // ---- L0: ground truth from a language server ---------------------------
    // Deliberately ahead of `local_only`: the server's answer outranks every
    // heuristic, including the analyzer's own same-file scope walk.
    match rf.precise_status.as_deref() {
        Some("hit") => {
            if let Some(id) = rf.precise_symbol_id {
                return Some((id, rf.precise_confidence.unwrap_or(1.0) as f32));
            }
            // The target file was re-analyzed after the precise pass ran, so the
            // id was dropped. Fall through and let the heuristics answer until
            // the next `--precise` run refreshes it.
        }
        // The server resolved this reference, just not to a node in this graph:
        // the standard library, a third-party dependency, or a spot we keep no
        // symbol for. A heuristic guess here would be a false edge.
        Some("external") | Some("nonode") => return None,
        // `unresolved` (or never queried) — the server had nothing, so the
        // heuristic layers below are still the best available answer.
        _ => {}
    }

    // ---- L1: settled at sync time ------------------------------------------
    if rf.local_only {
        return None;
    }
    if let Some(id) = rf.resolved_symbol_id {
        return Some((id, rf.resolved_confidence.unwrap_or(0.95) as f32));
    }

    let rk = rf.receiver_kind.as_deref().unwrap_or("none");

    // ---- L2: precise import binding --------------------------------------
    if matches!(rk, "none" | "path")
        && let Some(Target::Symbol(id)) = ctx.binding.get(&(rf.file_id, rf.name.clone()))
    {
        return Some((*id, 0.90));
    }
    if matches!(rk, "path" | "value" | "self")
        && let Some(recv) = rf.receiver.as_deref()
    {
        // `foo.bar()` -> `foo`; `crate::a::mod::bar()` -> try `crate` and `mod`.
        let first = recv.split(['.', ':']).next().unwrap_or(recv);
        let last = recv.rsplit("::").next().unwrap_or(recv);
        for key in [first, last] {
            if let Some(Target::Module(fid)) = ctx.binding.get(&(rf.file_id, key.to_string()))
                && let Some(id) = exported_in(ctx, *fid, &rf.name)
            {
                return Some((id, 0.90));
            }
        }
    }

    // ---- L3: receiver-type resolution ---------------------------------------
    let inferred = infer_receiver_type(rf, ctx);
    if let Some((ty, conf)) = &inferred
        && let Some(id) = method_of_type(ctx, ty, rf, importer)
    {
        return Some((id, *conf));
    }

    // Rust and Go have no class inheritance, so a call on a receiver of known
    // type can only reach that type's own methods (or a trait's). Anything left
    // is `SystemTime::now()` / `node.walk()` on a type that is not ours.
    let nominal_typing = matches!(importer.language.as_str(), "rust" | "go");
    if nominal_typing && rk == "path" && names_foreign_type(rf, ctx) {
        return None;
    }

    // ---- L4: scored disambiguation ----------------------------------------
    let all = ctx.defs_by_name.get(rf.name.as_str())?;
    let cands: Vec<&SymbolRow> = all
        .iter()
        .filter(|s| {
            ctx.file_by_id
                .get(&s.file_id)
                .is_some_and(|f| languages_compatible(&f.language, &importer.language))
        })
        // A reference in type position names a type, never a function, even
        // though it carries no receiver and so looks like a bare call here.
        .filter(|s| match (rf.ref_kind.as_str(), rk) {
            ("type", _) => is_type_kind(&s.kind),
            // A name handed over as a value can be a function, a const, or a
            // unit struct / enum variant used as a constructor.
            ("value", _) => matches!(
                s.kind.as_str(),
                "function" | "method" | "variable" | "struct" | "enum"
            ),
            (_, "value") => s.kind == "method",
            (_, "none") => matches!(s.kind.as_str(), "function" | "method"),
            _ => true,
        })
        .filter(|s| match (&inferred, nominal_typing) {
            (Some((ty, _)), true) => belongs_to_receiver(ctx, s, ty),
            _ => true,
        })
        .copied()
        .collect();
    if cands.is_empty() {
        return None;
    }

    let reachable = ctx.imports_files.get(&rf.file_id);

    // `set.insert(x)` / `Default::default()` on a receiver we could not type is
    // almost always the standard library's method, not the one project function
    // that happens to share the name — and no import ties this file to it.
    if matches!(rk, "value" | "path")
        && STD_METHOD_NAMES.contains(&rf.name.as_str())
        && !cands
            .iter()
            .any(|c| reachable.is_some_and(|r| r.contains(&c.file_id)))
    {
        return None;
    }

    // The same reasoning one level up, for names in type position. A file
    // writing `Command` almost always means `std::process::Command`, not the
    // unrelated project enum that happens to share the name — unless an import
    // actually ties the two files together. Without this, adding type
    // references made every common type name a hub.
    if rf.ref_kind == "type"
        && STD_TYPE_NAMES.contains(&rf.name.as_str())
        && !cands
            .iter()
            .any(|c| reachable.is_some_and(|r| r.contains(&c.file_id)))
    {
        return None;
    }
    let importer_dir = dir_of(&importer.path);
    let score = |c: &SymbolRow| -> i32 {
        let mut sc = 0;
        if reachable.is_some_and(|r| r.contains(&c.file_id)) {
            sc += 3;
        }
        if c.is_exported {
            sc += 2;
        }
        match (rf.arg_count, c.param_count) {
            (Some(a), Some(p)) if a == p => sc += 2,
            (Some(a), Some(p)) if (a - p).abs() == 1 => sc += 1,
            _ => {}
        }
        if ctx
            .file_by_id
            .get(&c.file_id)
            .is_some_and(|f| dir_of(&f.path) == importer_dir)
        {
            sc += 1;
        }
        sc
    };

    let mut scored: Vec<(i32, &SymbolRow)> = cands.iter().map(|c| (score(c), *c)).collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    let best = scored[0];
    let margin = match scored.get(1) {
        Some(runner_up) => best.0 - runner_up.0,
        None => i32::MAX, // a single candidate is unambiguous
    };

    // "precision" mode: a tie produces no edge.
    if margin >= 2 {
        // The score says how well the winner fits; how alone it is says how far
        // to trust it. A name only one definition carries is a much safer guess
        // than the same score won against rivals, so it earns a higher ceiling.
        let base: f32 = 0.4 + 0.03 * best.0 as f32;
        let conf = if scored.len() == 1 {
            (base + 0.15).clamp(0.55, 0.75)
        } else if margin >= 4 {
            (base + 0.05).clamp(0.4, 0.7)
        } else {
            base.clamp(0.4, 0.7)
        };
        Some((best.1.id, conf))
    } else {
        None
    }
}

/// The method `rf.name` of type `ty` nearest the caller.
///
/// Type names are not unique across a project, so `ty` may name several
/// unrelated types. The caller's own file wins (a `self.walk()` belongs to the
/// type defined beside it), then its directory; failing both, the first
/// definition is kept, which is what a single-candidate lookup always did.
fn method_of_type(ctx: &ResolveCtx, ty: &str, rf: &RefRow, importer: &FileRow) -> Option<i64> {
    let ids = ctx.type_methods.get(ty)?.get(&rf.name)?;
    if let [only] = ids.as_slice() {
        return Some(*only);
    }
    let file_of = |id: &i64| ctx.sym_by_id.get(id).and_then(|s| ctx.file_by_id.get(&s.file_id));
    let importer_dir = dir_of(&importer.path);
    ids.iter()
        .find(|id| file_of(id).is_some_and(|f| f.id == importer.id))
        .or_else(|| {
            ids.iter()
                .find(|id| file_of(id).is_some_and(|f| dir_of(&f.path) == importer_dir))
        })
        .or_else(|| ids.first())
        .copied()
}

/// Whether candidate `c` could be the method a call on a receiver of type `ty`
/// reaches: it is one of that type's methods, a trait's (default methods live on
/// the trait, not the implementor), or has no recorded owner to contradict.
fn belongs_to_receiver(ctx: &ResolveCtx, c: &SymbolRow, ty: &str) -> bool {
    let Some(owner) = c.type_name.as_deref() else {
        return true;
    };
    let (owner, ty) = (base_type(owner), base_type(ty));
    owner == ty
        || ctx.defs_by_name.get(owner).is_some_and(|defs| {
            defs.iter()
                .any(|d| matches!(d.kind.as_str(), "trait" | "interface"))
        })
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
    "Command", "Error", "Result", "Option", "Path", "PathBuf", "Duration", "File", "Entry",
    "Builder", "Handle", "Sender", "Receiver", "Config", "Context", "State", "Instant", "Range",
    "Output", "Child", "Args", "Event", "Message", "Request", "Response", "Value", "Node",
    "Parser", "Formatter", "Writer", "Reader", "Iter", "Item", "Key", "Id", "Name", "Type",
];

/// Method names the standard library (and every collection type) already owns.
/// A same-named project method is only credible when the file imports it.
const STD_METHOD_NAMES: &[&str] = &[
    "insert", "push", "pop", "get", "remove", "contains", "extend", "default", "new", "len",
    "iter", "next", "clone", "into", "from", "map", "join", "trim", "split", "parse", "read",
    "write", "open", "close", "add", "set", "clear", "first", "last", "find", "filter",
];

fn exported_in(ctx: &ResolveCtx, file_id: i64, name: &str) -> Option<i64> {
    let list = ctx.syms_by_file.get(&file_id)?;
    list.iter()
        .find(|s| s.name == name && s.is_exported)
        .or_else(|| list.iter().find(|s| s.name == name))
        .map(|s| s.id)
}

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
    } else if let Some(ty) = from.and_then(|f| ctx.local_types.get(&(f, first.to_string()))) {
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
    matches!(
        kind,
        "struct" | "enum" | "trait" | "interface" | "class" | "type" | "component"
    )
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

fn languages_compatible(a: &str, b: &str) -> bool {
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
