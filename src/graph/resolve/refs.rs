//! Resolve a reference (call / use of a name) to the symbol it most likely
//! points at, with a confidence score.

use std::collections::HashMap;

use crate::cache::models::{FileRow, RefRow, SymbolRow};

pub(super) fn resolve_ref(
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

    // Only consider definitions in files of compatible language.
    let all = defs_by_name.get(rf.name.as_str())?;
    let list: Vec<&&SymbolRow> = all
        .iter()
        .filter(|s| {
            file_by_id
                .get(&s.file_id)
                .is_some_and(|f| languages_compatible(&f.language, &importer.language))
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

fn languages_compatible(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let is_js_family = |lang: &str| matches!(lang, "javascript" | "typescript" | "vue" | "svelte");
    is_js_family(a) && is_js_family(b)
}
