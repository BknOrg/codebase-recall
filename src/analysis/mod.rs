//! AST-based extraction of symbols, imports, and references from source files.

pub mod lang;
mod javascript;
mod python;
mod rust;

pub use lang::Language;

use crate::cache::models::{NewImport, NewRef, NewSymbol};

/// Everything an analyzer extracts from a single file.
#[derive(Debug, Default)]
pub struct ParsedFile {
    /// Declared symbols, ordered outermost-first (parents before children).
    pub symbols: Vec<NewSymbol>,
    pub imports: Vec<NewImport>,
    pub refs: Vec<NewRef>,
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
    }
}
