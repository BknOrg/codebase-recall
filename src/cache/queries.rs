use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::CacheDb;
use super::mappers::{
    REF_COLUMNS, map_binding_row, map_file_row, map_import_row, map_ref_row, map_scope_row,
    map_string_literal_row, map_symbol_row,
};
use super::models::{
    BindingRow, FileRow, ImportRow, RefRow, ScopeRow, StringLiteralRow, SymbolRow,
};
use super::utils::levenshtein;

impl CacheDb {
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// `(symbols, imports)` currently held in the cache, whatever the last sync touched.
    pub fn totals(&self) -> Result<(usize, usize)> {
        let count = |table: &str| -> Result<usize> {
            let n: i64 = self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(n as usize)
        };
        Ok((count("symbols")?, count("imports")?))
    }

    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, path, language, content_hash, mtime, size, parsed_ok, precise_synced_at
             FROM files",
        )?;
        let rows = stmt
            .query_map([], map_file_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_symbols(&self) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, file_id, name, kind, parent_symbol_id, is_exported,
                    start_line, end_line, start_byte, end_byte, signature,
                    param_count, type_name
             FROM symbols",
        )?;
        let rows = stmt
            .query_map([], map_symbol_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Look up a tracked file's row by its project-relative, `/`-normalized path.
    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, language, content_hash, mtime, size, parsed_ok, precise_synced_at
                 FROM files WHERE path = ?1",
                params![path],
                map_file_row,
            )
            .optional()?)
    }

    /// Symbols in `file_id` whose [start_line, end_line] overlaps [start_line, end_line],
    /// ordered smallest-range-first so the tightest enclosing symbol comes first.
    pub fn symbols_overlapping_lines(
        &self,
        file_id: i64,
        start_line: i64,
        end_line: i64,
    ) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, file_id, name, kind, parent_symbol_id, is_exported,
                    start_line, end_line, start_byte, end_byte, signature,
                    param_count, type_name
             FROM symbols
             WHERE file_id = ?1
               AND start_line IS NOT NULL AND end_line IS NOT NULL
               AND start_line <= ?3 AND end_line >= ?2
             ORDER BY (end_line - start_line) ASC",
        )?;
        let rows = stmt
            .query_map(params![file_id, start_line, end_line], map_symbol_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Search symbols by name or signature with optional kind and exported filter.
    pub fn search_symbols(
        &self,
        query: &str,
        kind_filter: Option<&str>,
        exported_only: bool,
        limit: usize,
    ) -> Result<Vec<(SymbolRow, String)>> {
        let pattern = format!("%{query}%");
        let mut sql = String::from(
            "SELECT s.id, s.file_id, s.name, s.kind, s.parent_symbol_id, s.is_exported,
                    s.start_line, s.end_line, s.start_byte, s.end_byte, s.signature,
                    s.param_count, s.type_name, f.path
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE (s.name LIKE ?1 OR s.signature LIKE ?1)",
        );

        if let Some(k) = kind_filter {
            sql.push_str(&format!(" AND s.kind = '{}'", k.replace('\'', "''")));
        }
        if exported_only {
            sql.push_str(" AND s.is_exported = 1");
        }
        sql.push_str(" ORDER BY (s.name = ?2) DESC, LENGTH(s.name) ASC, s.name ASC LIMIT ?3");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![pattern, query, limit as i64], |r| {
                Ok((
                    SymbolRow {
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
                    },
                    r.get::<_, String>(13)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Search string literals appearing as arguments in calls or macros.
    pub fn search_string_literals(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(StringLiteralRow, String)>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare_cached(
            "SELECT sl.id, sl.file_id, sl.value, sl.callee, sl.line, f.path
             FROM string_literals sl
             JOIN files f ON sl.file_id = f.id
             WHERE sl.value LIKE ?1
             ORDER BY (sl.value = ?2) DESC, LENGTH(sl.value) ASC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![pattern, query, limit as i64], |r| {
                Ok((map_string_literal_row(r)?, r.get::<_, String>(5)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Fuzzy search symbols by Levenshtein distance against known symbol names.
    pub fn fuzzy_search_symbols(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(SymbolRow, String, usize)>> {
        let all = self.all_symbols()?;
        let files = self.all_files()?;
        let file_map: HashMap<i64, String> = files.into_iter().map(|f| (f.id, f.path)).collect();

        let q_lower = query.to_ascii_lowercase();
        let mut scored: Vec<(SymbolRow, String, usize)> = Vec::new();

        for s in all {
            let s_lower = s.name.to_ascii_lowercase();
            let dist = levenshtein(&q_lower, &s_lower);
            let max_len = q_lower.len().max(s_lower.len());
            if dist <= 3 || (max_len > 4 && dist <= (max_len * 2 / 5)) || s_lower.contains(&q_lower) {
                let path = file_map.get(&s.file_id).cloned().unwrap_or_default();
                scored.push((s, path, dist));
            }
        }

        scored.sort_by_key(|(_, _, dist)| *dist);
        scored.truncate(limit);
        Ok(scored)
    }

    pub fn all_imports(&self) -> Result<Vec<ImportRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, file_id, raw_specifier, imported_name, alias, is_relative, start_line
             FROM imports",
        )?;
        let rows = stmt
            .query_map([], map_import_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_refs(&self) -> Result<Vec<RefRow>> {
        let mut stmt = self.conn.prepare_cached(&format!("{REF_COLUMNS} FROM refs"))?;
        let rows = stmt
            .query_map([], map_ref_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every ref recorded for one file, in source order.
    pub fn refs_in_file(&self, file_id: i64) -> Result<Vec<RefRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("{REF_COLUMNS} FROM refs WHERE file_id = ?1 ORDER BY id"))?;
        let rows = stmt
            .query_map([file_id], map_ref_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_scopes(&self) -> Result<Vec<ScopeRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, file_id, parent_scope_id, owner_symbol_id, kind, start_byte, end_byte
             FROM scopes",
        )?;
        let rows = stmt
            .query_map([], map_scope_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn all_bindings(&self) -> Result<Vec<BindingRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, file_id, scope_id, name, binding_kind, symbol_id, import_id, type_expr
             FROM bindings",
        )?;
        let rows = stmt
            .query_map([], map_binding_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
