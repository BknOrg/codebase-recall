//! bkndb table definitions for the cache.
//!
//! The cache is derived data: whenever [`SCHEMA_VERSION`] changes (or the file
//! was created by an older layout), every table is dropped and recreated, and
//! the next sync re-analyzes every file.

use anyhow::Result;
use bkndb::relational::{ColumnKind, ColumnSchema, TableSchema};

/// Bump whenever any table below changes shape.
pub const SCHEMA_VERSION: i64 = 6;

/// `cache_meta` key that records the [`SCHEMA_VERSION`] the file was built with.
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Not `meta`: bkndb reserves that name for its own bookkeeping.
pub const META: &str = "cache_meta";
pub const FILES: &str = "files";
pub const SYMBOLS: &str = "symbols";
pub const IMPORTS: &str = "imports";
pub const REFS: &str = "refs";
pub const SCOPES: &str = "scopes";
pub const BINDINGS: &str = "bindings";
pub const STRING_LITERALS: &str = "string_literals";

/// Every table the cache owns, built once per [`super::CacheDb`].
pub struct Schemas {
    pub meta: TableSchema,
    pub files: TableSchema,
    pub symbols: TableSchema,
    pub imports: TableSchema,
    pub refs: TableSchema,
    pub scopes: TableSchema,
    pub bindings: TableSchema,
    pub string_literals: TableSchema,
}

use ColumnKind::{Bool, Float, Int, Str};

/// An auto-increment `id` table; nullable columns, `indexes` on the lookup keys.
fn table(name: &str, columns: &[(&str, ColumnKind)], indexes: &[&str]) -> Result<TableSchema> {
    let mut b = TableSchema::builder(name).column(ColumnSchema::new("id", Int));
    for (col, kind) in columns {
        b = b.column(ColumnSchema::new(*col, *kind));
    }
    b = b.primary_key("id").auto_increment();
    for idx in indexes {
        b = b.index(*idx);
    }
    Ok(b.build()?)
}

impl Schemas {
    pub fn build() -> Result<Self> {
        let meta = TableSchema::builder(META)
            .column(ColumnSchema::new("key", Str))
            .column(ColumnSchema::new("value", Str))
            .primary_key("key")
            .build()?;

        let files = table(
            FILES,
            &[
                ("path", Str),
                ("language", Str),
                ("content_hash", Str),
                ("mtime", Int),
                ("size", Int),
                ("parsed_ok", Bool),
                ("updated_at", Int),
                ("precise_synced_at", Int),
            ],
            &["path"],
        )?;

        let symbols = table(
            SYMBOLS,
            &[
                ("file_id", Int),
                ("name", Str),
                ("kind", Str),
                ("parent_symbol_id", Int),
                ("is_exported", Bool),
                ("start_line", Int),
                ("end_line", Int),
                ("start_byte", Int),
                ("end_byte", Int),
                ("signature", Str),
                ("param_count", Int),
                ("type_name", Str),
            ],
            &["file_id"],
        )?;

        let imports = table(
            IMPORTS,
            &[
                ("file_id", Int),
                ("raw_specifier", Str),
                ("imported_name", Str),
                ("alias", Str),
                ("is_relative", Bool),
                ("start_line", Int),
            ],
            &["file_id"],
        )?;

        let refs = table(
            REFS,
            &[
                ("file_id", Int),
                ("from_symbol_id", Int),
                ("name", Str),
                ("ref_kind", Str),
                ("receiver", Str),
                ("start_line", Int),
                ("arg_count", Int),
                ("receiver_kind", Str),
                ("local_only", Bool),
                ("resolved_symbol_id", Int),
                ("resolved_confidence", Float),
                ("name_start_byte", Int),
                ("precise_symbol_id", Int),
                ("precise_confidence", Float),
                ("precise_status", Str),
            ],
            &["file_id"],
        )?;

        let scopes = table(
            SCOPES,
            &[
                ("file_id", Int),
                ("parent_scope_id", Int),
                ("owner_symbol_id", Int),
                ("kind", Str),
                ("start_byte", Int),
                ("end_byte", Int),
            ],
            &["file_id"],
        )?;

        let bindings = table(
            BINDINGS,
            &[
                ("file_id", Int),
                ("scope_id", Int),
                ("name", Str),
                ("binding_kind", Str),
                ("symbol_id", Int),
                ("import_id", Int),
                ("type_expr", Str),
            ],
            &["file_id"],
        )?;

        let string_literals = table(
            STRING_LITERALS,
            &[("file_id", Int), ("value", Str), ("callee", Str), ("line", Int)],
            &["file_id"],
        )?;

        Ok(Self {
            meta,
            files,
            symbols,
            imports,
            refs,
            scopes,
            bindings,
            string_literals,
        })
    }

    /// Every table, parents before children.
    pub fn all(&self) -> [&TableSchema; 8] {
        [
            &self.meta,
            &self.files,
            &self.symbols,
            &self.imports,
            &self.refs,
            &self.scopes,
            &self.bindings,
            &self.string_literals,
        ]
    }
}
