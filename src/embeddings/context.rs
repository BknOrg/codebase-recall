//! Bounded symbol-purpose views. Source chunks remain independent and intact.
use super::Embedder;
use crate::{analysis::embedding_bodies::Bodies, cache::models::SymbolRow};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const VERSION: &str = "symbol-purpose-v2";

pub fn marker(
    db: &crate::cache::CacheDb,
    file: &crate::cache::models::FileRow,
    key: &str,
    budget: usize,
) -> Result<String> {
    Ok(format!(
        "{VERSION}:{budget}:{key}:{}:{}",
        file.content_hash,
        db.meta_get(&format!("analyzer:{}", file.path))?
            .unwrap_or_default()
    ))
}

pub fn pending(
    db: &crate::cache::CacheDb,
    project: &std::path::Path,
    selection: &crate::selection::Selection,
    budget: usize,
) -> Result<Vec<(crate::cache::models::FileRow, String)>> {
    let cfg = crate::config::ProjectConfig::load(project)?;
    let key = super::config_key(&cfg.embeddings);
    let mut files = vec![];
    for file in super::scope(db, &cfg)?
        .into_iter()
        .filter(|f| f.parsed_ok && selection.accepts(&f.path))
    {
        let expected = marker(db, &file, &key, budget)?;
        if db.purpose_marker(file.id)?.as_deref() != Some(expected.as_str()) {
            files.push((file, expected));
        }
    }
    files.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    Ok(files)
}

pub fn refresh(
    db: &crate::cache::CacheDb,
    project: &std::path::Path,
    selection: &crate::selection::Selection,
    model: &mut dyn Embedder,
    budget: usize,
) -> Result<serde_json::Value> {
    let cfg = crate::config::ProjectConfig::load(project)?;
    let key = super::config_key(&cfg.embeddings);
    let files = pending(db, project, selection, budget)?;
    let mut inferred = 0;
    let mut reused = 0;
    let mut stored = 0;
    crate::progress::phase("purpose_context_index");
    for (file, expected) in &files {
        crate::operation::check()?;
        let source = db.source_snapshot(file)?;
        let symbols = db.symbols_for_file(file.id)?;
        let tree = crate::analysis::embedding_bodies::parse_tree(&file.path, &source);
        let bodies = tree.as_ref().map(crate::analysis::embedding_bodies::bodies);
        let facts = tree
            .as_ref()
            .map(|t| syntax(&source, &symbols, t))
            .unwrap_or_default();
        let mut rows = vec![];
        let mut missing = vec![];
        for symbol in &symbols {
            if let Some(context) = build(
                &file.path,
                &file.content_hash,
                &source,
                symbol,
                &symbols,
                bodies.as_ref(),
                facts.get(&symbol.id),
                model,
                &key,
                budget,
            )? {
                if let Some(vector) = db.purpose_vector(&context.input_hash, &key, &context.text)? {
                    ensure!(
                        vector.len() == cfg.embeddings.dimensions
                            && vector.iter().all(|v| v.is_finite())
                            && vector.iter().any(|v| *v != 0.0),
                        "invalid cached purpose vector; run embeddings sync --full"
                    );
                    reused += 1;
                    rows.push((context, vector));
                } else {
                    missing.push(context);
                }
            }
        }
        let mut ordered = vec![];
        for c in missing {
            ordered.push((model.input_tokens(&c.text)?, c));
        }
        ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.symbol_key.cmp(&b.1.symbol_key)));
        for batch in ordered.chunks(cfg.embeddings.batch_size.max(1)) {
            crate::operation::check()?;
            let vectors = model.embed(batch.iter().map(|(_, c)| c.text.clone()).collect())?;
            ensure!(
                vectors.len() == batch.len(),
                "purpose embedding returned wrong count"
            );
            for ((_, c), v) in batch.iter().zip(vectors) {
                rows.push((c.clone(), super::normalize(&v, cfg.embeddings.dimensions)?));
                inferred += 1;
            }
        }
        crate::operation::check()?;
        db.commit_purpose_file(file, &key, expected, &rows)?;
        stored += rows.len();
    }
    Ok(
        serde_json::json!({"model":super::MODEL,"version":VERSION,"context_tokens":budget,"updated_files":files.len(),"embedded_contexts":inferred,"reused_contexts":reused,"stored_contexts":stored,"base_vectors_reembedded":0}),
    )
}

pub fn valid_vectors(
    db: &crate::cache::CacheDb,
    project: &std::path::Path,
    budget: usize,
) -> Result<std::collections::HashMap<String, Vec<f32>>> {
    let cfg = crate::config::ProjectConfig::load(project)?;
    let key = super::config_key(&cfg.embeddings);
    let mut files = std::collections::HashMap::new();
    for file in super::scope(db, &cfg)?.into_iter().filter(|f| f.parsed_ok) {
        if db.purpose_marker(file.id)?.as_deref() == Some(marker(db, &file, &key, budget)?.as_str())
        {
            files.insert(file.path, file.content_hash);
        }
    }
    Ok(db
        .purpose_rows()?
        .into_iter()
        .filter(|r| {
            r.config_key == key && files.get(&r.context.path) == Some(&r.context.source_hash)
        })
        .map(|r| (r.context.symbol_key, r.vector))
        .collect())
}

#[derive(Default, Clone)]
pub struct Facts {
    pub structure: Vec<&'static str>,
    pub operations: Vec<String>,
}

/// Whole-function AST evidence, independent of query text and function labels.
/// Rare operations get space before repetitive plumbing; their source order is
/// retained after selection. String/comment text cannot create fake call nodes.
pub fn syntax(
    source: &str,
    symbols: &[SymbolRow],
    tree: &tree_sitter::Tree,
) -> std::collections::HashMap<i64, Facts> {
    use std::collections::{HashMap, HashSet};
    let mut all = vec![];
    let mut frequency = HashMap::<String, usize>::new();
    for symbol in symbols.iter().filter(|s| {
        matches!(
            s.kind.as_str(),
            "function" | "method" | "constructor" | "destructor"
        )
    }) {
        let (Some(start), Some(end)) = (symbol.start_byte, symbol.end_byte) else {
            continue;
        };
        let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
            continue;
        };
        if start >= end || source.get(start..end).is_none() {
            continue;
        }
        let Some(root) = tree.root_node().descendant_for_byte_range(start, end - 1) else {
            continue;
        };
        let mut pending = vec![root];
        let mut operations = vec![];
        let mut structure = vec![];
        while let Some(node) = pending.pop() {
            let kind = node.kind();
            let text = source.get(node.byte_range()).unwrap_or("");
            if matches!(kind, "try_expression") && !structure.contains(&"error propagation") {
                structure.push("error propagation");
            }
            if (matches!(kind, "closure_expression" | "lambda")
                || (kind == "type_identifier" && matches!(text, "Fn" | "FnOnce" | "FnMut")))
                && !structure.contains(&"closure")
            {
                structure.push("closure");
            }
            if matches!(
                kind,
                "for_expression"
                    | "for_statement"
                    | "while_expression"
                    | "while_statement"
                    | "for_in_statement"
            ) && !structure.contains(&"loop")
            {
                structure.push("loop");
            }
            if matches!(
                kind,
                "call_expression" | "call" | "method_invocation" | "object_creation_expression"
            ) {
                let target = node
                    .child_by_field_name("function")
                    .or_else(|| node.child_by_field_name("name"))
                    .or_else(|| node.named_child(0));
                if let Some(target) = target {
                    let name = source.get(target.byte_range()).unwrap_or("").trim();
                    if !name.is_empty() && name.len() <= 96 && !name.contains('\n') {
                        let short = name
                            .rsplit(['.', ':'])
                            .find(|s| !s.is_empty())
                            .unwrap_or(name)
                            .to_owned();
                        operations.push((node.start_byte(), short, format!("call {name}")));
                    }
                }
            }
            if matches!(
                kind,
                "string_content" | "string_fragment" | "interpreted_string_literal_content"
            ) && !text.is_empty()
                && text.len() <= 48
                && text
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_$.-".contains(c))
            {
                operations.push((node.start_byte(), text.into(), format!("tag {text}")));
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
        operations.sort_by_key(|o| o.0);
        let mut unique = HashSet::new();
        operations.retain(|o| unique.insert(o.1.clone()));
        for (_, term, _) in &operations {
            *frequency.entry(term.clone()).or_default() += 1;
        }
        all.push((symbol.id, structure, operations));
    }
    all.into_iter()
        .map(|(id, structure, mut operations)| {
            operations.sort_by_key(|o| (*frequency.get(&o.1).unwrap_or(&0), o.0));
            operations.truncate(16);
            operations.sort_by_key(|o| o.0);
            (
                id,
                Facts {
                    structure,
                    operations: operations.into_iter().map(|o| o.2).collect(),
                },
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Ranking {
    pub function_body_weight: f64,
    pub container_body_weight: f64,
    pub query_view_weight: f64,
    #[serde(default)]
    pub purpose_query_view_weight: Option<f64>,
}
impl Default for Ranking {
    fn default() -> Self {
        Self {
            function_body_weight: 0.75,
            container_body_weight: 0.75,
            query_view_weight: 0.25,
            purpose_query_view_weight: None,
        }
    }
}

impl Ranking {
    pub fn score(&self, kind: &str, evidence: [Option<f64>; 4]) -> Option<f64> {
        let blend = |a: Option<f64>, b: Option<f64>, weight: f64| match (a, b) {
            (Some(a), Some(b)) => Some(a * (1.0 - weight) + b * weight),
            (a, b) => a.or(b),
        };
        let body = blend(evidence[0], evidence[1], self.query_view_weight);
        let purpose = blend(
            evidence[2],
            evidence[3],
            self.purpose_query_view_weight
                .unwrap_or(self.query_view_weight),
        );
        let body_weight = if container(kind) {
            self.container_body_weight
        } else {
            self.function_body_weight
        };
        blend(purpose, body, body_weight)
    }
}

pub use crate::cache::purpose::Context;

pub fn container(kind: &str) -> bool {
    matches!(
        kind,
        "concept" | "class" | "struct" | "interface" | "trait" | "enum" | "impl" | "type"
    )
}

fn owner(symbol: &SymbolRow, symbols: &[SymbolRow]) -> String {
    let mut id = symbol.parent_symbol_id;
    let mut names = vec![];
    let mut seen = std::collections::HashSet::new();
    while let Some(parent) = id {
        if !seen.insert(parent) {
            break;
        }
        let Some(s) = symbols.iter().find(|s| s.id == parent) else {
            break;
        };
        names.push(s.name.as_str());
        id = s.parent_symbol_id;
    }
    names.reverse();
    names.join("::")
}

fn append_unit(text: &mut String, unit: &str, model: &dyn Embedder, budget: usize) -> Result<bool> {
    if unit.trim().is_empty() {
        return Ok(true);
    }
    let next = format!("{text}\n{unit}");
    if model.input_tokens(&next)? > budget {
        return Ok(false);
    }
    *text = next;
    Ok(true)
}

fn docstring<'a>(source: &'a str, start: usize, end: usize, bodies: Option<&Bodies>) -> &'a str {
    let Some(&(lo, hi)) = bodies
        .and_then(|b| b.get(&(start, end)))
        .and_then(|v| v.first())
    else {
        return "";
    };
    let Some(body) = source.get(lo..hi) else {
        return "";
    };
    let body = body.trim_start();
    let body = body.strip_prefix(['r', 'u', 'R', 'U']).unwrap_or(body);
    let marker = if body.starts_with("\"\"\"") {
        "\"\"\""
    } else if body.starts_with("'''") {
        "'''"
    } else {
        return "";
    };
    let Some(finish) = body[3..].find(marker) else {
        return "";
    };
    &body[..finish + 6]
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    path: &str,
    source_hash: &str,
    source: &str,
    symbol: &SymbolRow,
    symbols: &[SymbolRow],
    bodies: Option<&Bodies>,
    facts: Option<&Facts>,
    model: &dyn Embedder,
    config_key: &str,
    budget: usize,
) -> Result<Option<Context>> {
    if !container(&symbol.kind)
        && !matches!(
            symbol.kind.as_str(),
            "function" | "method" | "constructor" | "destructor"
        )
    {
        return Ok(None);
    }
    ensure!(
        (64..=510).contains(&budget),
        "purpose context budget must be 64..=510 tokens"
    );
    let (Some(start), Some(end)) = (symbol.start_byte, symbol.end_byte) else {
        return Ok(None);
    };
    let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
        return Ok(None);
    };
    if start >= end || source.get(start..end).is_none() {
        return Ok(None);
    }
    let owner = owner(symbol, symbols);
    let qualified = if owner.is_empty() {
        symbol.name.clone()
    } else {
        format!("{owner}::{}", symbol.name)
    };
    let mut module = path.rsplit('/').take(3).collect::<Vec<_>>();
    module.reverse();
    let mut text = format!(
        "title: {qualified} | text: {}\nfile: {}",
        symbol.kind,
        module.join("/")
    );
    let signature = symbol
        .signature
        .as_deref()
        .unwrap_or_else(|| source[start..end].lines().next().unwrap_or(""));
    // Declaration and documentation are whole units whenever they fit. Long
    // names cannot consume the entire source/documentation budget.
    let prefix = model.offsets(&text)?;
    if prefix.len() > budget / 2 {
        let finish = prefix[budget / 2 - 1].1;
        ensure!(
            text.is_char_boundary(finish),
            "invalid context token boundary"
        );
        text.truncate(finish);
    }
    append_unit(&mut text, signature, model, budget)?;
    let doc_start = crate::cache::content::doc_start(source, start);
    let docs = format!(
        "{}\n{}",
        &source[doc_start..start],
        docstring(source, start, end, bodies)
    );
    // Split at paragraphs rather than chopping a fixed-width source window.
    let doc_budget = if facts.is_some() || container(&symbol.kind) {
        budget * 2 / 3
    } else {
        budget
    };
    for paragraph in docs.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if !append_unit(&mut text, paragraph, model, doc_budget)? {
            for line in paragraph.lines() {
                if !append_unit(&mut text, line, model, doc_budget)? {
                    break;
                }
            }
            break;
        }
    }
    if let Some(facts) = facts {
        if !facts.structure.is_empty() {
            append_unit(
                &mut text,
                &format!("structure: {}", facts.structure.join("; ")),
                model,
                budget,
            )?;
        }
        for operation in &facts.operations {
            if !append_unit(&mut text, operation, model, budget)? {
                break;
            }
        }
    }
    if container(&symbol.kind) {
        for member in symbols.iter().filter(|s| {
            s.parent_symbol_id == Some(symbol.id)
                || (symbol.kind == "struct" && s.type_name.as_deref() == Some(symbol.name.as_str()))
        }) {
            let declaration = member.signature.as_deref().unwrap_or(&member.name);
            if !append_unit(
                &mut text,
                &format!("member {}: {declaration}", member.kind),
                model,
                budget,
            )? {
                break;
            }
        }
    }
    ensure!(
        model.input_tokens(&text)? <= budget,
        "purpose context exceeded its token budget"
    );
    let input_hash = blake3::hash(format!("{config_key}\n{text}").as_bytes())
        .to_hex()
        .to_string();
    Ok(Some(Context {
        symbol_key: super::chunk::symbol_key(path, symbol),
        path: path.into(),
        name: symbol.name.clone(),
        kind: symbol.kind.to_string(),
        source_hash: source_hash.into(),
        input_hash,
        text,
    }))
}

#[cfg(all(test, feature = "embeddings"))]
#[path = "context_evaluation.rs"]
mod evaluation;
