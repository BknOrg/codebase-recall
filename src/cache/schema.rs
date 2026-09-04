//! SQLite schema and forward-only migrations for the graph cache.
//!
//! `PRAGMA user_version` tracks the applied schema version. Each entry in
//! [`MIGRATIONS`] moves the database from version `i` to version `i + 1`.

/// Current schema version. Must equal `MIGRATIONS.len()`.
pub const SCHEMA_VERSION: i64 = 1;

/// Ordered migration scripts. `MIGRATIONS[0]` upgrades v0 -> v1, etc.
pub const MIGRATIONS: &[&str] = &[V1];

const V1: &str = r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);

CREATE TABLE files (
    id           INTEGER PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,      -- project-relative, unix-normalized
    language     TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mtime        INTEGER,
    size         INTEGER,
    parsed_ok    INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER
);

CREATE TABLE symbols (
    id               INTEGER PRIMARY KEY,
    file_id          INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL,          -- function|method|class|struct|enum|interface|type|variable
    parent_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    is_exported      INTEGER NOT NULL DEFAULT 0,
    start_line       INTEGER,
    end_line         INTEGER,
    start_byte       INTEGER,
    end_byte         INTEGER,
    signature        TEXT
);

CREATE TABLE imports (
    id            INTEGER PRIMARY KEY,
    file_id       INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    raw_specifier TEXT NOT NULL,
    imported_name TEXT,
    alias         TEXT,
    is_relative   INTEGER NOT NULL DEFAULT 0,
    start_line    INTEGER
);

CREATE TABLE refs (
    id             INTEGER PRIMARY KEY,
    file_id        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    from_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    ref_kind       TEXT NOT NULL,            -- call|read|write|type
    receiver       TEXT,
    start_line     INTEGER
);

CREATE INDEX idx_symbols_file ON symbols(file_id);
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_imports_file ON imports(file_id);
CREATE INDEX idx_refs_file    ON refs(file_id);
"#;
