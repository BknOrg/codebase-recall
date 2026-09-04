//! Plain row structs mirroring the cache tables.
//!
//! Some fields are not read yet but are kept so the row types stay a faithful
//! mirror of the schema for future consumers.
#![allow(dead_code)]

/// A tracked source file.
#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub language: String,
    pub content_hash: String,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
    pub parsed_ok: bool,
}

/// A declared symbol (function, method, type, module-level variable, ...).
#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: String,
    pub parent_symbol_id: Option<i64>,
    pub is_exported: bool,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub start_byte: Option<i64>,
    pub end_byte: Option<i64>,
    pub signature: Option<String>,
}

/// One imported binding (`imported_name` is `None` for whole-module imports).
#[derive(Debug, Clone)]
pub struct ImportRow {
    pub id: i64,
    pub file_id: i64,
    pub raw_specifier: String,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub is_relative: bool,
    pub start_line: Option<i64>,
}

/// A reference to a name (call / read / write / type position).
#[derive(Debug, Clone)]
pub struct RefRow {
    pub id: i64,
    pub file_id: i64,
    pub from_symbol_id: Option<i64>,
    pub name: String,
    pub ref_kind: String,
    pub receiver: Option<String>,
    pub start_line: Option<i64>,
}

/// Symbol payload produced by an analyzer, before it has a database id.
#[derive(Debug, Clone)]
pub struct NewSymbol {
    pub name: String,
    pub kind: String,
    /// Index into the same file's symbol list identifying the lexical parent.
    pub parent_index: Option<usize>,
    pub is_exported: bool,
    pub start_line: i64,
    pub end_line: i64,
    pub start_byte: i64,
    pub end_byte: i64,
    pub signature: Option<String>,
}

/// Import payload produced by an analyzer.
#[derive(Debug, Clone)]
pub struct NewImport {
    pub raw_specifier: String,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub is_relative: bool,
    pub start_line: i64,
}

/// Reference payload produced by an analyzer.
#[derive(Debug, Clone)]
pub struct NewRef {
    pub name: String,
    pub ref_kind: String,
    pub receiver: Option<String>,
    pub start_line: i64,
    pub start_byte: i64,
}
