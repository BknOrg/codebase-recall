use anyhow::Result;
use rusqlite::params;

use super::CacheDb;
use super::models::{
    NewBinding, NewImport, NewRef, NewScope, NewStringLiteral, NewSymbol, PreciseStatus,
};
use super::utils::{innermost_symbol, unix_now};

impl CacheDb {
    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Remove a file and all its analysis rows (cascades).
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Store one file's language-server answers and stamp the file as done.
    ///
    /// Written as a single transaction so a run that is interrupted half way
    /// leaves the file un-stamped and it simply gets re-queried next time.
    pub fn set_precise_results(
        &mut self,
        file_id: i64,
        results: &[(i64, PreciseStatus, Option<i64>, f64)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upd = tx.prepare_cached(
                "UPDATE refs
                    SET precise_status = ?2, precise_symbol_id = ?3, precise_confidence = ?4
                  WHERE id = ?1",
            )?;
            for (ref_id, status, symbol_id, confidence) in results {
                let conf = matches!(status, PreciseStatus::Hit).then_some(*confidence);
                upd.execute(params![ref_id, status.as_str(), symbol_id, conf])?;
            }
        }
        tx.execute(
            "UPDATE files SET precise_synced_at = ?2 WHERE id = ?1",
            params![file_id, unix_now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Forget every language-server answer for the given files, so the next
    /// `--precise` run asks again from scratch.
    pub fn clear_precise(&mut self, file_ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut clear_refs = tx.prepare_cached(
                "UPDATE refs
                    SET precise_status = NULL, precise_symbol_id = NULL, precise_confidence = NULL
                  WHERE file_id = ?1",
            )?;
            let mut clear_file =
                tx.prepare_cached("UPDATE files SET precise_synced_at = NULL WHERE id = ?1")?;
            for id in file_ids {
                clear_refs.execute([id])?;
                clear_file.execute([id])?;
            }
        }
        tx.commit()?;
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
        string_literals: &[NewStringLiteral],
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
                 updated_at = excluded.updated_at,
                 -- the file changed, so any language-server answers for it are
                 -- stale and must be asked again on the next --precise run
                 precise_synced_at = NULL",
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
        tx.execute("DELETE FROM string_literals WHERE file_id = ?1", [file_id])?;

        // Pass 1: insert symbols without parents, remember ids + ranges.
        let mut ids: Vec<i64> = Vec::with_capacity(symbols.len());
        {
            let mut ins = tx.prepare_cached(
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
            let mut upd = tx.prepare_cached("UPDATE symbols SET parent_symbol_id = ?1 WHERE id = ?2")?;
            for (i, s) in symbols.iter().enumerate() {
                if let Some(pidx) = s.parent_index
                    && let Some(&pid) = ids.get(pidx)
                {
                    upd.execute(params![pid, ids[i]])?;
                }
            }
        }

        // Scopes: pass 1 insert (parent left null), pass 2 wire parent ids.
        let mut scope_ids: Vec<i64> = Vec::with_capacity(scopes.len());
        {
            let mut ins = tx.prepare_cached(
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
            let mut upd = tx.prepare_cached("UPDATE scopes SET parent_scope_id = ?1 WHERE id = ?2")?;
            for (i, sc) in scopes.iter().enumerate() {
                if let Some(pidx) = sc.parent_index
                    && let Some(&pid) = scope_ids.get(pidx)
                {
                    upd.execute(params![pid, scope_ids[i]])?;
                }
            }
        }

        // Imports (before bindings so `namespace` bindings can point at them).
        let mut import_ids: Vec<i64> = Vec::with_capacity(imports.len());
        {
            let mut ins = tx.prepare_cached(
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
            let mut ins = tx.prepare_cached(
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
            let mut ins = tx.prepare_cached(
                "INSERT INTO refs(file_id, from_symbol_id, name, ref_kind, receiver, start_line,
                                  arg_count, receiver_kind, local_only,
                                  resolved_symbol_id, resolved_confidence, name_start_byte)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                    rf.name_start_byte,
                ])?;
            }
        }

        // String literals
        {
            let mut ins = tx.prepare_cached(
                "INSERT INTO string_literals(file_id, value, callee, line)
                 VALUES(?1, ?2, ?3, ?4)",
            )?;
            for sl in string_literals {
                ins.execute(params![file_id, sl.value, sl.callee, sl.line])?;
            }
        }

        tx.commit()?;
        Ok(())
    }
}
