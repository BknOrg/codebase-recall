use std::collections::HashMap;

use anyhow::Result;
use bkndb::relational::{Query, Row, TableSchema};
use bkndb::value::PropValue;

use super::CacheDb;
use super::mappers::{
    binding_row, file_row, import_row, ref_row, scope_row, string_literal_row, symbol_row,
};
use super::models::{
    BindingRow, FileRow, ImportRow, RefRow, ScopeRow, StringLiteralRow, SymbolRow,
};
use super::utils::levenshtein;

/// Case-insensitive (ASCII) substring test; `needle_lower` must already be lowercase.
fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle_lower)
}

impl CacheDb {
    /// Run `query` against `table` on one consistent snapshot.
    pub(crate) fn rows(&self, table: &TableSchema, query: &Query) -> Result<Vec<Row>> {
        Ok(self
            .db
            .read_tx(|r| r.relational().table(table.clone()).find(query))?)
    }

    fn rows_where_file(&self, table: &TableSchema, file_id: i64) -> Result<Vec<Row>> {
        self.rows(table, &Query::new().where_eq("file_id", file_id))
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let row = self
            .db
            .read_tx(|r| r.relational().table(self.tables.meta.clone()).get(&PropValue::from(key)))?;
        Ok(row.and_then(|r| match r.values.get("value") {
            Some(PropValue::Str(v)) => Some(v.clone()),
            _ => None,
        }))
    }

    /// `(symbols, imports)` currently held in the cache, whatever the last sync touched.
    pub fn totals(&self) -> Result<(usize, usize)> {
        let (symbols, imports) = self.db.read_tx(|r| {
            let rel = r.relational();
            let symbols = rel.table(self.tables.symbols.clone()).count(&Query::new())?;
            let imports = rel.table(self.tables.imports.clone()).count(&Query::new())?;
            Ok((symbols, imports))
        })?;
        Ok((symbols, imports))
    }

    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let rows = self.rows(&self.tables.files, &Query::new())?;
        Ok(rows.iter().map(file_row).collect())
    }

    pub fn all_symbols(&self) -> Result<Vec<SymbolRow>> {
        let rows = self.rows(&self.tables.symbols, &Query::new())?;
        Ok(rows.iter().map(symbol_row).collect())
    }

    /// Look up a tracked file's row by its project-relative, `/`-normalized path.
    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        let rows = self.rows(&self.tables.files, &Query::new().where_eq("path", path))?;
        Ok(rows.first().map(file_row))
    }

    /// Symbols in `file_id` whose [start_line, end_line] overlaps [start_line, end_line],
    /// ordered smallest-range-first so the tightest enclosing symbol comes first.
    pub fn symbols_overlapping_lines(
        &self,
        file_id: i64,
        start_line: i64,
        end_line: i64,
    ) -> Result<Vec<SymbolRow>> {
        let mut hits: Vec<(i64, SymbolRow)> = self
            .rows_where_file(&self.tables.symbols, file_id)?
            .iter()
            .map(symbol_row)
            .filter_map(|s| {
                let (lo, hi) = (s.start_line?, s.end_line?);
                (lo <= end_line && hi >= start_line).then_some((hi - lo, s))
            })
            .collect();
        hits.sort_by_key(|(width, _)| *width);
        Ok(hits.into_iter().map(|(_, s)| s).collect())
    }

    /// Search symbols by name or signature with optional kind and exported filter.
    pub fn search_symbols(
        &self,
        query: &str,
        kind_filter: Option<&str>,
        exported_only: bool,
        limit: usize,
    ) -> Result<Vec<(SymbolRow, String)>> {
        let needle = query.to_ascii_lowercase();
        let paths: HashMap<i64, String> = self.all_files()?.into_iter().map(|f| (f.id, f.path)).collect();

        let mut hits: Vec<SymbolRow> = self
            .all_symbols()?
            .into_iter()
            .filter(|s| {
                (contains_ci(&s.name, &needle)
                    || s.signature.as_deref().is_some_and(|sig| contains_ci(sig, &needle)))
                    && kind_filter.is_none_or(|k| s.kind == k)
                    && (!exported_only || s.is_exported)
                    && paths.contains_key(&s.file_id)
            })
            .collect();

        // Exact name first, then shortest name, then alphabetical.
        hits.sort_by(|a, b| {
            (a.name != query, a.name.chars().count(), &a.name)
                .cmp(&(b.name != query, b.name.chars().count(), &b.name))
        });
        hits.truncate(limit);
        Ok(hits
            .into_iter()
            .map(|s| {
                let path = paths.get(&s.file_id).cloned().unwrap_or_default();
                (s, path)
            })
            .collect())
    }

    /// Search string literals appearing as arguments in calls or macros.
    pub fn search_string_literals(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(StringLiteralRow, String)>> {
        let needle = query.to_ascii_lowercase();
        let paths: HashMap<i64, String> = self.all_files()?.into_iter().map(|f| (f.id, f.path)).collect();

        let rows = self.rows(&self.tables.string_literals, &Query::new())?;
        let mut hits: Vec<StringLiteralRow> = rows
            .iter()
            .map(string_literal_row)
            .filter(|sl| contains_ci(&sl.value, &needle) && paths.contains_key(&sl.file_id))
            .collect();

        hits.sort_by_key(|sl| (sl.value != query, sl.value.chars().count()));
        hits.truncate(limit);
        Ok(hits
            .into_iter()
            .map(|sl| {
                let path = paths.get(&sl.file_id).cloned().unwrap_or_default();
                (sl, path)
            })
            .collect())
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
        let rows = self.rows(&self.tables.imports, &Query::new())?;
        Ok(rows.iter().map(import_row).collect())
    }

    pub fn all_refs(&self) -> Result<Vec<RefRow>> {
        let rows = self.rows(&self.tables.refs, &Query::new())?;
        Ok(rows.iter().map(ref_row).collect())
    }

    /// Every ref recorded for one file, in source order.
    pub fn refs_in_file(&self, file_id: i64) -> Result<Vec<RefRow>> {
        let rows = self.rows(
            &self.tables.refs,
            &Query::new().where_eq("file_id", file_id).order_by_asc("id"),
        )?;
        Ok(rows.iter().map(ref_row).collect())
    }

    pub fn all_scopes(&self) -> Result<Vec<ScopeRow>> {
        let rows = self.rows(&self.tables.scopes, &Query::new())?;
        Ok(rows.iter().map(scope_row).collect())
    }

    pub fn all_bindings(&self) -> Result<Vec<BindingRow>> {
        let rows = self.rows(&self.tables.bindings, &Query::new())?;
        Ok(rows.iter().map(binding_row).collect())
    }
}
