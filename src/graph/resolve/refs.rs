//! Layered resolution of a reference (call / use of a name) to the symbol it
//! points at, with a confidence score.
//!
//! The layers, tried most-certain first:
//!
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
    /// type simple name -> { method name -> symbol id }.
    pub type_methods: &'a HashMap<String, HashMap<String, i64>>,
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
    // ---- L1: settled at sync time ------------------------------------------
    if rf.local_only {
        return None;
    }
    if let Some(id) = rf.resolved_symbol_id {
        return Some((id, rf.resolved_confidence.unwrap_or(0.95) as f32));
    }

    let rk = rf.receiver_kind.as_deref().unwrap_or("none");

    // ---- L2: precise import binding --------------------------------------
    if matches!(rk, "none" | "path") {
        if let Some(Target::Symbol(id)) = ctx.binding.get(&(rf.file_id, rf.name.clone())) {
            return Some((*id, 0.90));
        }
    }
    if matches!(rk, "path" | "value" | "self") {
        if let Some(recv) = rf.receiver.as_deref() {
            // `foo.bar()` -> `foo`; `crate::a::mod::bar()` -> try `crate` and `mod`.
            let first = recv.split(['.', ':']).next().unwrap_or(recv);
            let last = recv.rsplit("::").next().unwrap_or(recv);
            for key in [first, last] {
                if let Some(Target::Module(fid)) = ctx.binding.get(&(rf.file_id, key.to_string())) {
                    if let Some(id) = exported_in(ctx, *fid, &rf.name) {
                        return Some((id, 0.90));
                    }
                }
            }
        }
    }

    // ---- L3: receiver-type resolution ---------------------------------------
    if let Some((ty, conf)) = infer_receiver_type(rf, ctx) {
        if let Some(&id) = ctx.type_methods.get(&ty).and_then(|m| m.get(&rf.name)) {
            return Some((id, conf));
        }
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
        .filter(|s| match rk {
            "value" => s.kind == "method",
            "none" => matches!(s.kind.as_str(), "function" | "method"),
            _ => true,
        })
        .copied()
        .collect();
    if cands.is_empty() {
        return None;
    }

    let reachable = ctx.imports_files.get(&rf.file_id);
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
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let best = scored[0];
    let margin = match scored.get(1) {
        Some(runner_up) => best.0 - runner_up.0,
        None => i32::MAX, // a single candidate is unambiguous
    };

    // "precision" mode: a tie produces no edge.
    if margin >= 2 {
        let conf = (0.4 + 0.03 * best.0 as f32).clamp(0.4, 0.7);
        Some((best.1.id, conf))
    } else {
        None
    }
}

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
    ctx.defs_by_name.get(name).is_some_and(|list| {
        list.iter().any(|s| {
            matches!(
                s.kind.as_str(),
                "struct" | "enum" | "trait" | "interface" | "class" | "type" | "component"
            )
        })
    })
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
