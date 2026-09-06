//! Resolve an import specifier to a project-relative file path (per language),
//! and derive the display label for an unresolved external module.

use std::collections::HashSet;

use crate::cache::models::ImportRow;

pub(super) fn resolve_import(
    lang_group: &str,
    importer_rel: &str,
    im: &ImportRow,
    path_set: &HashSet<String>,
) -> Option<String> {
    match lang_group {
        "rust" => resolve_rust_import(importer_rel, &im.raw_specifier, path_set),
        "javascript" | "typescript" | "vue" | "svelte" => {
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
    const EXTS: &[&str] = &[
        "", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".d.ts", ".vue", ".svelte",
    ];
    const INDEX: &[&str] = &[
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
        "/index.mjs",
        "/index.vue",
        "/index.svelte",
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

pub(super) fn external_root(spec: &str, lang_group: &str) -> String {
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
