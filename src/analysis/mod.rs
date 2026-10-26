//! AST-based extraction of symbols, imports, and references from source files.

pub(crate) mod embedding_bodies;
mod go;
mod java;
mod javascript;
mod kotlin;
mod kotlin_comments;
pub mod lang;
pub mod python;
mod rust;
pub mod scope;
mod sfc;
mod toml;

pub use lang::Language;

use crate::cache::models::{NewBinding, NewImport, NewRef, NewScope, NewStringLiteral, NewSymbol};

/// Everything an analyzer extracts from a single file.
#[derive(Debug, Default)]
pub struct ParsedFile {
    /// Declared symbols, ordered outermost-first (parents before children).
    pub symbols: Vec<NewSymbol>,
    pub imports: Vec<NewImport>,
    pub refs: Vec<NewRef>,
    /// Lexical scopes, ordered outermost-first (parents before children).
    pub scopes: Vec<NewScope>,
    /// Names introduced in each scope (locals, params, fields, imports, ...).
    pub bindings: Vec<NewBinding>,
    /// String literals extracted from call arguments or macros.
    pub string_literals: Vec<NewStringLiteral>,
    /// False if the parser reported syntax errors or the language is unsupported.
    pub parse_ok: bool,
    pub diagnostics: Vec<serde_json::Value>,
}

/// Parse `source` according to `language`.
pub fn parse_file(language: Language, source: &str) -> ParsedFile {
    match language {
        Language::Rust => rust::parse(source),
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            javascript::parse(source, language)
        }
        Language::Python => python::python::parse(source),
        Language::Java => java::parse(source),
        Language::Kotlin => kotlin::parse(source),
        Language::Vue | Language::Svelte => sfc::parse(source, language),
        Language::Go => go::parse(source),
        Language::Toml => toml::parse(source),
    }
}

/// Extracts a single-line declaration signature from a tree-sitter AST node,
/// stopping before the node's body block if present and trimming trailing delimiters.
pub(crate) fn node_signature(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let start = node.start_byte();
    let end = if let Some(body) = node.child_by_field_name("body") {
        body.start_byte()
    } else if let Some(body) = node.child_by_field_name("block") {
        body.start_byte()
    } else {
        let mut cursor = node.walk();
        let body_child = node.children(&mut cursor).find(|c| {
            matches!(
                c.kind(),
                "block"
                    | "function_body"
                    | "class_body"
                    | "interface_body"
                    | "statement_block"
                    | "compound_statement"
            )
        });
        if let Some(b) = body_child {
            b.start_byte()
        } else {
            node.end_byte()
        }
    };
    if end > start && end <= src.len() {
        let s = std::str::from_utf8(&src[start..end]).ok()?;
        let trimmed = s.trim().trim_end_matches(['{', ';', ':']).trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
        }
    } else {
        None
    }
}

/// Bounded syntax diagnostics preserve original byte and line locations.
pub fn diagnostics(root: tree_sitter::Node<'_>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.is_error() || node.is_missing() {
            out.push(serde_json::json!({"kind":node.kind(),"missing":node.is_missing(),"line":node.start_position().row+1,"column_bytes":node.start_position().column,"start_byte":node.start_byte(),"end_byte":node.end_byte()}));
            if out.len() >= 20 {
                break;
            }
        }
        let mut cursor = node.walk();
        pending.extend(
            node.children(&mut cursor)
                .filter(|n| n.has_error() || n.is_error() || n.is_missing()),
        );
    }
    out.sort_by_key(|d| d["start_byte"].as_u64());
    out
}
