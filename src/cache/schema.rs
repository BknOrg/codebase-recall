//! SQLite schema and forward-only migrations for the graph cache.
//!
//! `PRAGMA user_version` tracks the applied schema version. Each entry in
//! [`MIGRATIONS`] moves the database from version `i` to version `i + 1`.

/// Current schema version. Must equal `MIGRATIONS.len()`.
pub const SCHEMA_VERSION: i64 = 4;

/// Ordered migration scripts. `MIGRATIONS[0]` upgrades v0 -> v1, etc.
pub const MIGRATIONS: &[&str] = &[V1, V2, V3, V4];

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

/// v2 — scope tree + bindings (a compact, SCIP-shaped symbol table) plus a few
/// denormalized columns that the layered resolver reads.
///
/// * `scopes`  — one lexical scope per node (module / fn / class / block).
/// * `bindings` — every name introduced in a scope, with an optional declared
///   type. `binding_kind = 'field'` rows are the "type composition" data:
///   `struct Foo { bar: Bar }` yields a field binding `bar` with `type_expr = "Bar"`.
/// * `refs.local_only` — the ref resolves to a local/param inside its own file,
///   so it must NOT become a cross-symbol edge.
/// * `refs.resolved_symbol_id` — same-file scope resolution, computed at sync.
const V2: &str = r#"
CREATE TABLE scopes (
    id              INTEGER PRIMARY KEY,
    file_id         INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    parent_scope_id INTEGER REFERENCES scopes(id) ON DELETE CASCADE,
    owner_symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,           -- module|function|method|class|struct|block
    start_byte      INTEGER NOT NULL,
    end_byte        INTEGER NOT NULL
);

CREATE TABLE bindings (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    scope_id     INTEGER NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    binding_kind TEXT NOT NULL,              -- local|param|field|symbol|import|namespace
    symbol_id    INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    import_id    INTEGER REFERENCES imports(id) ON DELETE SET NULL,
    type_expr    TEXT
);

CREATE INDEX idx_scopes_file   ON scopes(file_id);
CREATE INDEX idx_bindings_file  ON bindings(file_id);
CREATE INDEX idx_bindings_scope ON bindings(scope_id);

ALTER TABLE refs ADD COLUMN arg_count          INTEGER;
ALTER TABLE refs ADD COLUMN receiver_kind      TEXT;      -- none|path|value|self
ALTER TABLE refs ADD COLUMN local_only         INTEGER NOT NULL DEFAULT 0;
ALTER TABLE refs ADD COLUMN resolved_symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL;
ALTER TABLE refs ADD COLUMN resolved_confidence REAL;

ALTER TABLE symbols ADD COLUMN param_count INTEGER;
ALTER TABLE symbols ADD COLUMN type_name   TEXT;          -- method: owning type's simple name
"#;

/// v3 — ground-truth resolution from a real language server (`sync --precise`).
///
/// * `refs.name_start_byte` — byte offset of the *name token* of the reference
///   (`bar` in `foo.bar()`), which is where a `textDocument/definition` request
///   has to point. The whole-expression offset would resolve `foo` instead.
/// * `refs.precise_status` — what the server answered:
///   `hit` (a definition inside the project, `precise_symbol_id` is set),
///   `external` (a definition outside the project tree — stdlib or a dependency),
///   `nonode` (inside the project but at a place we keep no symbol node for),
///   `unresolved` (the server had no answer). `NULL` means never queried.
///   Only `hit` yields an edge; `external`/`nonode` deliberately yield none,
///   which is how precise mode removes the false edges heuristics would invent.
/// * `files.precise_synced_at` — when this file's refs were last queried. Reset
///   to `NULL` whenever the file is re-analyzed, so stale answers are re-asked.
///
/// Existing rows predate `name_start_byte`, and unchanged files are never
/// re-parsed, so the file table is cleared to force one full re-analysis. Only
/// the derived cache is dropped; nothing in the working tree is touched.
const V3: &str = r#"
ALTER TABLE refs ADD COLUMN name_start_byte    INTEGER;
ALTER TABLE refs ADD COLUMN precise_symbol_id  INTEGER REFERENCES symbols(id) ON DELETE SET NULL;
ALTER TABLE refs ADD COLUMN precise_confidence REAL;
ALTER TABLE refs ADD COLUMN precise_status     TEXT;      -- hit|external|nonode|unresolved

ALTER TABLE files ADD COLUMN precise_synced_at INTEGER;

CREATE INDEX idx_refs_precise ON refs(file_id, precise_status);

DELETE FROM files;
"#;

/// v4 — string literals / config keys indexed from call arguments and macros.
const V4: &str = r#"
CREATE TABLE string_literals (
    id      INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    value   TEXT NOT NULL,
    callee  TEXT,
    line    INTEGER
);

CREATE INDEX idx_strings_val  ON string_literals(value);
CREATE INDEX idx_strings_file ON string_literals(file_id);
"#;
