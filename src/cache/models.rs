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
    /// Declared parameter count for functions/methods (`None` if not applicable).
    pub param_count: Option<i64>,
    /// For a `method`, the simple name of the type that owns it.
    pub type_name: Option<String>,
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
    pub arg_count: Option<i64>,
    /// `none` | `path` | `value` | `self`
    pub receiver_kind: Option<String>,
    /// The ref binds to a local/param in its own file — not a cross-symbol edge.
    pub local_only: bool,
    /// Same-file scope resolution result, computed at sync time.
    pub resolved_symbol_id: Option<i64>,
    pub resolved_confidence: Option<f64>,
}

/// A lexical scope. `id`/`parent_scope_id`/`owner_symbol_id` are database ids.
#[derive(Debug, Clone)]
pub struct ScopeRow {
    pub id: i64,
    pub file_id: i64,
    pub parent_scope_id: Option<i64>,
    pub owner_symbol_id: Option<i64>,
    pub kind: String,
    pub start_byte: i64,
    pub end_byte: i64,
}

/// A name introduced in a scope, with an optional declared type.
#[derive(Debug, Clone)]
pub struct BindingRow {
    pub id: i64,
    pub file_id: i64,
    pub scope_id: i64,
    pub name: String,
    /// `local` | `param` | `field` | `symbol` | `import` | `namespace`
    pub binding_kind: String,
    pub symbol_id: Option<i64>,
    pub import_id: Option<i64>,
    pub type_expr: Option<String>,
}

/// Symbol payload produced by an analyzer, before it has a database id.
#[derive(Debug, Clone, Default)]
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
    pub param_count: Option<i64>,
    pub type_name: Option<String>,
}

/// Import payload produced by an analyzer.
#[derive(Debug, Clone, Default)]
pub struct NewImport {
    pub raw_specifier: String,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub is_relative: bool,
    pub start_line: i64,
}

/// Reference payload produced by an analyzer.
#[derive(Debug, Clone, Default)]
pub struct NewRef {
    pub name: String,
    pub ref_kind: String,
    pub receiver: Option<String>,
    pub start_line: i64,
    pub start_byte: i64,
    pub arg_count: Option<i64>,
    /// `none` | `path` | `value` | `self` (defaults to `none`).
    pub receiver_kind: String,
    /// Set by the analyzer's local-resolution post-pass.
    pub local_only: bool,
    /// Index into the same file's symbol list, when the ref resolves in-file.
    pub resolved_local_symbol_index: Option<usize>,
}

/// Scope payload produced by an analyzer, before it has a database id.
#[derive(Debug, Clone, Default)]
pub struct NewScope {
    /// Index into the same file's scope list identifying the parent scope.
    pub parent_index: Option<usize>,
    /// Index into the same file's symbol list this scope is the body of, if any.
    pub owner_symbol_index: Option<usize>,
    pub kind: String,
    pub start_byte: i64,
    pub end_byte: i64,
}

/// Binding payload produced by an analyzer, before it has a database id.
#[derive(Debug, Clone, Default)]
pub struct NewBinding {
    /// Index into the same file's scope list.
    pub scope_index: usize,
    pub name: String,
    pub binding_kind: String,
    /// Index into the same file's symbol list (for `symbol` bindings).
    pub symbol_index: Option<usize>,
    /// Index into the same file's import list (for `import` / `namespace`).
    pub import_index: Option<usize>,
    pub type_expr: Option<String>,
}
