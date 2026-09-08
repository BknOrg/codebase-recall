//! SQLite-backed graph cache stored at `<project>/.code-ctx/cache.db`.

pub mod models;
pub mod schema;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

use models::{
    BindingRow, FileRow, ImportRow, NewBinding, NewImport, NewRef, NewScope, NewSymbol, RefRow,
    ScopeRow, SymbolRow,
};

/// Directory that holds all code-rcl project state.
pub const CODE_CTX_DIR: &str = ".code-rcl";
/// Cache database file name inside [`CODE_CTX_DIR`].
pub const DB_FILE: &str = "cache.db";

pub struct CacheDb {
    conn: Connection,
}

/// Absolute path to `<project>/.code-rcl`.
pub fn ctx_dir(project_root: &Path) -> PathBuf {
    project_root.join(CODE_CTX_DIR)
}

/// Absolute path to `<project>/.code-rcl/cache.db`.
pub fn db_path(project_root: &Path) -> PathBuf {
    ctx_dir(project_root).join(DB_FILE)
}

impl CacheDb {
    /// Open (creating `.code-rcl/` and the database if needed) and migrate.
    pub fn open(project_root: &Path) -> Result<Self> {
        let dir = ctx_dir(project_root);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let conn = Connection::open(db_path(project_root))
            .with_context(|| format!("opening {}", db_path(project_root).display()))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;

        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        let mut current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;

        while (current as usize) < schema::MIGRATIONS.len() {
            let script = schema::MIGRATIONS[current as usize];
            let tx = self.conn.transaction()?;
            tx.execute_batch(script)
                .with_context(|| format!("applying migration to v{}", current + 1))?;
            tx.commit()?;
            current += 1;
            self.conn.pragma_update(None, "user_version", current)?;
        }

        debug_assert_eq!(current, schema::SCHEMA_VERSION);
        Ok(())
    }

    // ----- meta -----------------------------------------------------------

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ----- reads --------------------------------------------------------

    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, content_hash, mtime, size, parsed_ok FROM files",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FileRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    language: r.get(2)?,
                    content_hash: r.get(3)?,
                    mtime: r.get(4)?,
                    size: r.get(5)?,
                    parsed_ok: r.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_symbols(&self) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, name, kind, parent_symbol_id, is_exported,
                    start_line, end_line, start_byte, end_byte, signature,
                    param_count, type_name
             FROM symbols",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SymbolRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    name: r.get(2)?,
                    kind: r.get(3)?,
                    parent_symbol_id: r.get(4)?,
                    is_exported: r.get::<_, i64>(5)? != 0,
                    start_line: r.get(6)?,
                    end_line: r.get(7)?,
                    start_byte: r.get(8)?,
                    end_byte: r.get(9)?,
                    signature: r.get(10)?,
                    param_count: r.get(11)?,
                    type_name: r.get(12)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_imports(&self) -> Result<Vec<ImportRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, raw_specifier, imported_name, alias, is_relative, start_line
             FROM imports",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ImportRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    raw_specifier: r.get(2)?,
                    imported_name: r.get(3)?,
                    alias: r.get(4)?,
                    is_relative: r.get::<_, i64>(5)? != 0,
                    start_line: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_refs(&self) -> Result<Vec<RefRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, from_symbol_id, name, ref_kind, receiver, start_line,
                    arg_count, receiver_kind, local_only, resolved_symbol_id, resolved_confidence
             FROM refs",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RefRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    from_symbol_id: r.get(2)?,
                    name: r.get(3)?,
                    ref_kind: r.get(4)?,
                    receiver: r.get(5)?,
                    start_line: r.get(6)?,
                    arg_count: r.get(7)?,
                    receiver_kind: r.get(8)?,
                    local_only: r.get::<_, i64>(9)? != 0,
                    resolved_symbol_id: r.get(10)?,
                    resolved_confidence: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_scopes(&self) -> Result<Vec<ScopeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, parent_scope_id, owner_symbol_id, kind, start_byte, end_byte
             FROM scopes",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ScopeRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    parent_scope_id: r.get(2)?,
                    owner_symbol_id: r.get(3)?,
                    kind: r.get(4)?,
                    start_byte: r.get(5)?,
                    end_byte: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_bindings(&self) -> Result<Vec<BindingRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, scope_id, name, binding_kind, symbol_id, import_id, type_expr
             FROM bindings",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BindingRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    scope_id: r.get(2)?,
                    name: r.get(3)?,
                    binding_kind: r.get(4)?,
                    symbol_id: r.get(5)?,
                    import_id: r.get(6)?,
                    type_expr: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ----- writes -------------------------------------------------------

    /// Remove a file and all its analysis rows (cascades).
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Replace all analysis rows for a single file in one transaction.
    ///
    /// `symbols` must be ordered outermost-first so `parent_index` always
    /// refers to an earlier entry. Each ref is attached to the innermost
    /// symbol whose byte range contains `ref.start_byte`.
    #[allow(clippy::too_many_arguments)]
    pub fn replace_file_analysis(
        &mut self,
        path: &str,
        language: &str,
        content_hash: &str,
        mtime: Option<i64>,
        size: Option<i64>,
        parsed_ok: bool,
        symbols: &[NewSymbol],
        imports: &[NewImport],
        refs: &[NewRef],
        scopes: &[NewScope],
        bindings: &[NewBinding],
    ) -> Result<()> {
        let now = unix_now();
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO files(path, language, content_hash, mtime, size, parsed_ok, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                 language = excluded.language,
                 content_hash = excluded.content_hash,
                 mtime = excluded.mtime,
                 size = excluded.size,
                 parsed_ok = excluded.parsed_ok,
                 updated_at = excluded.updated_at",
            params![
                path,
                language,
                content_hash,
                mtime,
                size,
                parsed_ok as i64,
                now
            ],
        )?;
        let file_id: i64 =
            tx.query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0))?;

        // Cascades clear symbols/imports/refs/scopes/bindings for this file.
        tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
        tx.execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;
        tx.execute("DELETE FROM refs WHERE file_id = ?1", [file_id])?;
        tx.execute("DELETE FROM scopes WHERE file_id = ?1", [file_id])?;
        tx.execute("DELETE FROM bindings WHERE file_id = ?1", [file_id])?;

        // Pass 1: insert symbols without parents, remember ids + ranges.
        let mut ids: Vec<i64> = Vec::with_capacity(symbols.len());
        {
            let mut ins = tx.prepare(
                "INSERT INTO symbols(file_id, name, kind, is_exported,
                                     start_line, end_line, start_byte, end_byte, signature,
                                     param_count, type_name)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for s in symbols {
                ins.execute(params![
                    file_id,
                    s.name,
                    s.kind,
                    s.is_exported as i64,
                    s.start_line,
                    s.end_line,
                    s.start_byte,
                    s.end_byte,
                    s.signature,
                    s.param_count,
                    s.type_name,
                ])?;
                ids.push(tx.last_insert_rowid());
            }
        }

        // Pass 2: wire parents.
        {
            let mut upd = tx.prepare("UPDATE symbols SET parent_symbol_id = ?1 WHERE id = ?2")?;
            for (i, s) in symbols.iter().enumerate() {
                if let Some(pidx) = s.parent_index {
                    if let Some(&pid) = ids.get(pidx) {
                        upd.execute(params![pid, ids[i]])?;
                    }
                }
            }
        }

        // Scopes: pass 1 insert (parent left null), pass 2 wire parent ids.
        let mut scope_ids: Vec<i64> = Vec::with_capacity(scopes.len());
        {
            let mut ins = tx.prepare(
                "INSERT INTO scopes(file_id, owner_symbol_id, kind, start_byte, end_byte)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for sc in scopes {
                let owner = sc.owner_symbol_index.and_then(|i| ids.get(i).copied());
                ins.execute(params![file_id, owner, sc.kind, sc.start_byte, sc.end_byte])?;
                scope_ids.push(tx.last_insert_rowid());
            }
        }
        {
            let mut upd = tx.prepare("UPDATE scopes SET parent_scope_id = ?1 WHERE id = ?2")?;
            for (i, sc) in scopes.iter().enumerate() {
                if let Some(pidx) = sc.parent_index {
                    if let Some(&pid) = scope_ids.get(pidx) {
                        upd.execute(params![pid, scope_ids[i]])?;
                    }
                }
            }
        }

        // Imports (before bindings so `namespace` bindings can point at them).
        let mut import_ids: Vec<i64> = Vec::with_capacity(imports.len());
        {
            let mut ins = tx.prepare(
                "INSERT INTO imports(file_id, raw_specifier, imported_name, alias, is_relative, start_line)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for im in imports {
                ins.execute(params![
                    file_id,
                    im.raw_specifier,
                    im.imported_name,
                    im.alias,
                    im.is_relative as i64,
                    im.start_line,
                ])?;
                import_ids.push(tx.last_insert_rowid());
            }
        }

        // Bindings.
        {
            let mut ins = tx.prepare(
                "INSERT INTO bindings(file_id, scope_id, name, binding_kind, symbol_id, import_id, type_expr)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for b in bindings {
                let Some(&scope_id) = scope_ids.get(b.scope_index) else {
                    continue;
                };
                let symbol_id = b.symbol_index.and_then(|i| ids.get(i).copied());
                let import_id = b.import_index.and_then(|i| import_ids.get(i).copied());
                ins.execute(params![
                    file_id,
                    scope_id,
                    b.name,
                    b.binding_kind,
                    symbol_id,
                    import_id,
                    b.type_expr,
                ])?;
            }
        }

        // Refs, attached to their enclosing symbol. `resolved_local_symbol_index`
        // (from the analyzer's scope walk) becomes a same-file `resolved_symbol_id`.
        {
            let mut ins = tx.prepare(
                "INSERT INTO refs(file_id, from_symbol_id, name, ref_kind, receiver, start_line,
                                  arg_count, receiver_kind, local_only,
                                  resolved_symbol_id, resolved_confidence)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for rf in refs {
                let enclosing = innermost_symbol(symbols, &ids, rf.start_byte);
                let resolved = rf.resolved_local_symbol_index.and_then(|i| ids.get(i).copied());
                let resolved_conf: Option<f64> = resolved.map(|_| 0.95);
                ins.execute(params![
                    file_id,
                    enclosing,
                    rf.name,
                    rf.ref_kind,
                    rf.receiver,
                    rf.start_line,
                    rf.arg_count,
                    rf.receiver_kind,
                    rf.local_only as i64,
                    resolved,
                    resolved_conf,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }
}

/// Innermost (narrowest) symbol whose byte range contains `byte`.
fn innermost_symbol(symbols: &[NewSymbol], ids: &[i64], byte: i64) -> Option<i64> {
    let mut best: Option<(i64, i64)> = None; // (width, id)
    for (i, s) in symbols.iter().enumerate() {
        if byte >= s.start_byte && byte < s.end_byte {
            let width = s.end_byte - s.start_byte;
            if best.map_or(true, |(w, _)| width < w) {
                best = Some((width, ids[i]));
            }
        }
    }
    best.map(|(_, id)| id)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
