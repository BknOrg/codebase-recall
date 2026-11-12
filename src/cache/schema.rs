//! bkndb table definitions for the cache.
//!
//! Schemas 10–12 preserve files and source snapshots while rebuilding derived
//! semantic indexes for schema 13.

use anyhow::Result;
use bkndb::relational::{ColumnKind, ColumnSchema, TableSchema};

/// Bump whenever any table below changes shape.
pub const SCHEMA_VERSION: i64 = 13;

/// `cache_meta` key that records the [`SCHEMA_VERSION`] the file was built with.
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Not `meta`: bkndb reserves that name for its own bookkeeping.
pub const META: &str = "cache_meta";
pub const EMBEDDING_CHUNKS: &str = "embedding_chunks";
pub const FILES: &str = "files";
pub const SOURCE_SNAPSHOTS: &str = "source_snapshots";
pub const SYMBOLS: &str = "symbols";
pub const IMPORTS: &str = "imports";
pub const REFS: &str = "refs";
pub const SCOPES: &str = "scopes";
pub const BINDINGS: &str = "bindings";
pub const STRING_LITERALS: &str = "string_literals";
pub const SEARCH_DOCUMENTS: &str = "search_documents";
pub const SEARCH_POSTINGS: &str = "search_postings";
pub const SEARCH_FILES: &str = "search_files";
pub const PURPOSE_CONTEXTS: &str = "purpose_contexts";
pub const PURPOSE_FILES: &str = "purpose_files";

/// Every table the cache owns, built once per [`super::CacheDb`].
pub struct Schemas {
    pub purpose_contexts: TableSchema,
    pub purpose_files: TableSchema,
    pub search_documents: TableSchema,
    pub search_postings: TableSchema,
    pub search_files: TableSchema,
    pub meta: TableSchema,
    pub embedding_chunks: TableSchema,
    pub files: TableSchema,
    pub source_snapshots: TableSchema,
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
        let purpose_contexts = TableSchema::builder(PURPOSE_CONTEXTS)
            .column(ColumnSchema::new("key", Str))
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("config_key", Str))
            .column(ColumnSchema::new("input_hash", Str))
            .column(ColumnSchema::new("payload", Str))
            .column(ColumnSchema::new("vector", ColumnKind::Bytes))
            .primary_key("key")
            .index("file_id")
            .index("input_hash")
            .build()?;
        let purpose_files = TableSchema::builder(PURPOSE_FILES)
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("marker", Str))
            .column(ColumnSchema::new("contexts", Int))
            .primary_key("file_id")
            .build()?;
        let search_documents = TableSchema::builder(SEARCH_DOCUMENTS)
            .column(ColumnSchema::new("key", Str))
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("length", Int))
            .column(ColumnSchema::new("payload", Str))
            .primary_key("key")
            .index("file_id")
            .build()?;
        let search_postings = TableSchema::builder(SEARCH_POSTINGS)
            .column(ColumnSchema::new("key", Str))
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("term", Str))
            .column(ColumnSchema::new("document", Str))
            .column(ColumnSchema::new("frequency", Int))
            .primary_key("key")
            .index("file_id")
            .index("term")
            .build()?;
        let search_files = TableSchema::builder(SEARCH_FILES)
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("marker", Str))
            .column(ColumnSchema::new("documents", Int))
            .column(ColumnSchema::new("length", Int))
            .primary_key("file_id")
            .build()?;
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

        let source_snapshots = TableSchema::builder(SOURCE_SNAPSHOTS)
            .column(ColumnSchema::new("file_id", Int))
            .column(ColumnSchema::new("source_hash", Str))
            .column(ColumnSchema::new("content", ColumnKind::Bytes))
            .primary_key("file_id")
            .build()?;

        let symbols = table(
            SYMBOLS,
            &[
                ("file_id", Int),
                ("name", Str),
                ("name_lower", Str),
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
                ("semantic", Str),
            ],
            &["file_id", "name_lower"],
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
                ("argument_types", Str),
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
                ("declaration_byte", Int),
                ("symbol_id", Int),
                ("import_id", Int),
                ("type_expr", Str),
            ],
            &["file_id"],
        )?;

        let string_literals = table(
            STRING_LITERALS,
            &[
                ("file_id", Int),
                ("value", Str),
                ("callee", Str),
                ("line", Int),
            ],
            &["file_id"],
        )?;

        let embedding_chunks = table(
            EMBEDDING_CHUNKS,
            &[
                ("path", Str),
                ("input_hash", Str),
                ("config_key", Str),
                ("source_hash", Str),
                ("valid", Bool),
                ("payload", Str),
                ("vector", ColumnKind::Bytes),
            ],
            &["path", "input_hash", "config_key"],
        )?;
        Ok(Self {
            purpose_contexts,
            purpose_files,
            search_documents,
            search_postings,
            search_files,
            embedding_chunks,
            meta,
            files,
            source_snapshots,
            symbols,
            imports,
            refs,
            scopes,
            bindings,
            string_literals,
        })
    }

    /// Every table, parents before children.
    pub fn all(&self) -> [&TableSchema; 15] {
        [
            &self.purpose_contexts,
            &self.purpose_files,
            &self.search_documents,
            &self.search_postings,
            &self.search_files,
            &self.meta,
            &self.embedding_chunks,
            &self.files,
            &self.source_snapshots,
            &self.symbols,
            &self.imports,
            &self.refs,
            &self.scopes,
            &self.bindings,
            &self.string_literals,
        ]
    }
}
