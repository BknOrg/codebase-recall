//! Balanced selective verification policy, independent of graph presentation.
use crate::{
    cache::{
        CacheDb,
        models::{FileRow, RefRow, SymbolRow},
    },
    graph::resolve::index::ReferenceIndex,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
pub const VERSION: &str = "selective-1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub reference: i64,
    pub file: i64,
    pub language: String,
    pub path: String,
    pub offset: Option<i64>,
    pub selected: bool,
    pub priority: u8,
    pub reasons: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub language: String,
    pub candidates: usize,
    pub cache_hits: usize,
    pub queried: usize,
    pub verified: usize,
    pub unresolved: usize,
    pub skipped: usize,
    pub failed: usize,
    pub pending: usize,
    pub server_starts: usize,
    pub server: Option<String>,
    pub reasons: Vec<String>,
    pub selection_reasons: std::collections::BTreeMap<String, usize>,
}
pub fn decisions(
    db: &CacheDb,
    files: &[FileRow],
    symbols: &[SymbolRow],
    references: &[RefRow],
) -> Result<Vec<Decision>> {
    let imports = db.all_imports()?;
    let scopes = db.all_scopes()?;
    let bindings = db.all_bindings()?;
    let index = ReferenceIndex::new(&db.project, files, symbols, &imports, &scopes, &bindings)?;
    let mut unsupported = HashSet::new();
    for file in files {
        if let Some(raw) = db.meta_get(&format!("diagnostics:{}", file.path))?
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
            && (value["parse_ok"] == false
                || value["errors"].as_array().is_some_and(|es| {
                    es.iter().any(|e| {
                        matches!(
                            e["kind"].as_str(),
                            Some("unsupported_construct" | "header_language_uncertain")
                        )
                    })
                }))
        {
            unsupported.insert(file.id);
        }
    }
    let mut overloads: HashMap<(&str, Option<&str>), usize> = HashMap::new();
    for s in symbols.iter().filter(|s| s.semantic.kind.callable()) {
        *overloads
            .entry((&s.name, s.semantic.owner.as_deref()))
            .or_default() += 1;
    }
    let owners: HashMap<_, _> = symbols
        .iter()
        .map(|s| (s.semantic.qualified_name.as_str(), s))
        .collect();
    let mut out = Vec::new();
    for r in references {
        crate::operation::check()?;
        let Some(file) = index.file_by_id.get(&r.file_id) else {
            continue;
        };
        if crate::analysis::lang::is_config_group(&file.language) {
            continue;
        }
        let resolution = index.heuristic(r);
        let target = resolution
            .as_ref()
            .and_then(|v| index.sym_by_id.get(&v.target));
        let source = r.from_symbol_id.and_then(|id| index.sym_by_id.get(&id));
        let mut reasons = Vec::new();
        let mut priority = 3;
        if !r.local_only {
            if resolution.as_ref().is_some_and(|v| v.ambiguous) {
                reasons.push("ambiguous".into());
                priority = 0;
            }
            if matches!(r.receiver_kind.as_deref(), Some("value" | "self"))
                && resolution
                    .as_ref()
                    .is_none_or(|v| v.ambiguous || v.evidence != "scope" && v.evidence != "import")
            {
                reasons.push("receiver_or_dispatch".into());
            }
            if let Some(target) = target {
                if target.semantic.kind.callable()
                    && overloads
                        .get(&(target.name.as_str(), target.semantic.owner.as_deref()))
                        .copied()
                        .unwrap_or(0)
                        > 1
                {
                    reasons.push("overload".into());
                }
                let owner = target
                    .semantic
                    .owner
                    .as_ref()
                    .and_then(|owner| owners.get(owner.as_str()).copied());
                let dynamic_method = target.kind == "method"
                    && target.semantic.modifiers.is_static != Some(true)
                    && matches!(
                        file.language.as_str(),
                        "java"
                            | "kotlin"
                            | "python"
                            | "javascript"
                            | "typescript"
                            | "vue"
                            | "svelte"
                    );
                if target.semantic.modifiers.is_virtual == Some(true)
                    || dynamic_method
                    || owner.is_some_and(|o| matches!(o.kind.as_str(), "interface" | "trait"))
                    || matches!(target.kind.as_str(), "interface" | "trait")
                {
                    reasons.push("dynamic_dispatch".into());
                }
            }
            for symbol in target.into_iter().chain(source) {
                if !symbol.semantic.type_parameters.is_empty() {
                    reasons.push("generic_dependent".into());
                }
                if symbol.kind == "macro"
                    || symbol
                        .semantic
                        .language_details
                        .get("conditional")
                        .and_then(|v| v.as_bool())
                        == Some(true)
                {
                    reasons.push("macro_or_conditional".into());
                }
            }
            if unsupported.contains(&file.id) {
                reasons.push("unsupported_lookup".into());
            }
            if !reasons.is_empty() {
                priority = priority.min(1);
            }
            if resolution.is_none() {
                reasons.push("unresolved".into());
                priority = priority.min(2);
            }
            if let Some(v) = &resolution
                && (v.confidence < 0.90 || !matches!(v.evidence.as_str(), "scope" | "import"))
            {
                reasons.push("weak_evidence".into());
            }
        }
        reasons.sort();
        reasons.dedup();
        let selected = !r.local_only && !reasons.is_empty();
        if !selected {
            reasons.push(
                if r.local_only {
                    "local_binding"
                } else {
                    "strong_evidence"
                }
                .into(),
            );
        }
        out.push(Decision {
            reference: r.id,
            file: r.file_id,
            language: file.language.clone(),
            path: file.path.clone(),
            offset: r.name_start_byte,
            selected,
            priority,
            reasons,
        });
    }
    out.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.path.cmp(&b.path))
            .then(a.offset.cmp(&b.offset))
            .then(a.reference.cmp(&b.reference))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(source: &str) -> (tempfile::TempDir, CacheDb) {
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), source).unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        (dir, db)
    }
    #[test]
    fn selection_uses_clean_heuristics_not_lsp_overlay_or_presentation() {
        let (_dir, mut db) = fixture(
            "fn strong(){} fn generic<T>(){} fn caller(){let 日本語=1; strong(); generic::<i32>(); missing(日本語);}",
        );
        let files = db.all_files().unwrap();
        let symbols = db.all_symbols().unwrap();
        let refs = db.all_refs().unwrap();
        let before = decisions(&db, &files, &symbols, &refs).unwrap();
        let named = |name: &str| {
            before
                .iter()
                .find(|d| refs.iter().any(|r| r.id == d.reference && r.name == name))
                .unwrap()
        };
        assert!(!named("strong").selected);
        assert!(
            named("generic")
                .reasons
                .contains(&"generic_dependent".into())
        );
        assert!(named("missing").reasons.contains(&"unresolved".into()));
        assert!(!named("日本語").selected);
        for d in &before {
            if let Some(offset) = d.offset {
                assert!(
                    std::fs::read_to_string(db.project.join(&d.path))
                        .unwrap()
                        .is_char_boundary(offset as usize)
                );
            }
        }
        let id = named("missing").reference;
        db.set_precise_subset(
            files[0].id,
            &[(id, crate::cache::models::PreciseStatus::External, None, 0.0)],
            false,
            &[],
        )
        .unwrap();
        let after = decisions(&db, &files, &symbols, &db.all_refs().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        assert!(db.all_files().unwrap()[0].precise_synced_at.is_none());
        assert!(
            db.all_refs()
                .unwrap()
                .iter()
                .any(|r| r.precise_status.is_none())
        );
    }
    #[test]
    fn legacy_precise_is_invalidated_without_reparsing_source() {
        let (_dir, mut db) = fixture("fn target(){} fn call(){target();missing();}");
        let files = db.all_files().unwrap();
        let refs = db.all_refs().unwrap();
        let symbols = db.all_symbols().unwrap();
        db.set_precise_results(
            files[0].id,
            &[(
                refs[0].id,
                crate::cache::models::PreciseStatus::External,
                None,
                0.0,
            )],
        )
        .unwrap();
        db.meta_set("precise_policy_version", "old").unwrap();
        super::super::state::prepare(&mut db).unwrap();
        assert!(
            db.all_refs()
                .unwrap()
                .iter()
                .all(|r| r.precise_status.is_none())
        );
        assert!(db.all_files().unwrap()[0].precise_synced_at.is_none());
        assert_eq!(
            db.all_files().unwrap()[0].content_hash,
            files[0].content_hash
        );
        assert_eq!(db.all_symbols().unwrap().len(), symbols.len());
    }
    #[test]
    fn interrupted_subset_without_fingerprint_is_not_reused_or_claimed_precise() {
        let (_dir, mut db) = fixture("fn call(){missing();}");
        let files = db.all_files().unwrap();
        let refs = db.all_refs().unwrap();
        db.meta_set("embedding_test_marker", "preserved").unwrap();
        db.set_precise_subset(
            files[0].id,
            &[(
                refs[0].id,
                crate::cache::models::PreciseStatus::External,
                None,
                0.0,
            )],
            false,
            &[],
        )
        .unwrap();
        assert_eq!(
            db.meta_get("precision_has_results").unwrap().as_deref(),
            Some("true")
        );
        super::super::state::prepare(&mut db).unwrap();
        assert!(
            db.all_refs()
                .unwrap()
                .iter()
                .all(|r| r.precise_status.is_none())
        );
        assert_eq!(
            db.meta_get("precision_has_results").unwrap().as_deref(),
            Some("false")
        );
        assert_eq!(
            db.meta_get("embedding_test_marker").unwrap().as_deref(),
            Some("preserved")
        );
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;
    #[test]
    fn strong_and_unknown_references_are_classified_in_every_source_family() {
        for (name, source) in [
            ("main.rs", "fn strong(){} fn run(){strong();unknown();}"),
            (
                "main.go",
                "package fixture\nfunc strong(){}\nfunc run(){strong();unknown()}\n",
            ),
            (
                "Main.java",
                "class Main {static void strong(){} static void run(){strong();unknown();}}",
            ),
            ("main.kt", "fun strong(){}\nfun run(){strong();unknown()}"),
            (
                "main.py",
                "def strong():\n    pass\ndef run():\n    strong()\n    unknown()\n",
            ),
            (
                "main.jsx",
                "function strong(){} function run(){strong();unknown();}",
            ),
            (
                "main.tsx",
                "function strong(){} function run(){strong();unknown();}",
            ),
            (
                "App.vue",
                "<script setup>function strong(){} function run(){strong();unknown();}</script><template><div/></template>",
            ),
            (
                "App.svelte",
                "<script>function strong(){} function run(){strong();unknown();}</script><div/>",
            ),
            (
                "main.c",
                "void strong(void){} void run(void){strong();unknown();}",
            ),
            (
                "main.cpp",
                "void strong(){} void run(){strong();unknown();}",
            ),
        ] {
            crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            )));
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(name), source).unwrap();
            let mut db = CacheDb::open(dir.path()).unwrap();
            crate::service::auto_sync(&mut db, dir.path()).unwrap();
            let refs = db.all_refs().unwrap();
            let decisions = decisions(
                &db,
                &db.all_files().unwrap(),
                &db.all_symbols().unwrap(),
                &refs,
            )
            .unwrap();
            for (name, expected) in [("strong", false), ("unknown", true)] {
                let r = refs
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap_or_else(|| panic!("{source}: reference {name} missing"));
                let d = decisions.iter().find(|d| d.reference == r.id).unwrap();
                assert_eq!(d.selected, expected, "{source}: {d:?}");
            }
        }
    }
    #[test]
    fn high_confidence_instance_method_still_requires_verification() {
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Main.java"),
            "class Main {void target(){} void run(){target();}}",
        )
        .unwrap();
        let mut db = CacheDb::open(dir.path()).unwrap();
        crate::service::auto_sync(&mut db, dir.path()).unwrap();
        let files = db.all_files().unwrap();
        let symbols = db.all_symbols().unwrap();
        let refs = db.all_refs().unwrap();
        let imports = db.all_imports().unwrap();
        let scopes = db.all_scopes().unwrap();
        let bindings = db.all_bindings().unwrap();
        let index = ReferenceIndex::new(dir.path(), &files, &symbols, &imports, &scopes, &bindings)
            .unwrap();
        // Inspect the real policy, including its language-specific dispatch rule.
        assert!(
            index
                .heuristic(refs.iter().find(|r| r.name == "target").unwrap())
                .unwrap()
                .confidence
                >= 0.90,
            "refs={refs:?}, bindings={bindings:?}, scopes={scopes:?}"
        );
        let selected = decisions(&db, &files, &symbols, &refs).unwrap();
        let d = selected
            .iter()
            .find(|d| {
                refs.iter()
                    .any(|r| r.id == d.reference && r.name == "target")
            })
            .unwrap();
        assert!(d.selected);
        assert!(d.reasons.contains(&"dynamic_dispatch".into()));
    }
}
