//! AST-based extraction of symbols, imports, and references from source files.

pub mod lang;
pub mod scope;
mod javascript;
mod python;
mod rust;
mod sfc;

pub use lang::Language;

use crate::cache::models::{NewBinding, NewImport, NewRef, NewScope, NewSymbol};

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
    /// False if the parser reported syntax errors or the language is unsupported.
    pub parse_ok: bool,
}

/// Parse `source` according to `language`.
pub fn parse_file(language: Language, source: &str) -> ParsedFile {
    match language {
        Language::Rust => rust::parse(source),
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            javascript::parse(source, language)
        }
        Language::Python => python::parse(source),
        Language::Vue | Language::Svelte => sfc::parse(source, language),
    }
}
