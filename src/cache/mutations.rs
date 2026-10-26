use anyhow::Result;
use bkndb::BknError;
use bkndb::value::{PropValue, Properties};

use super::CacheDb;
use super::models::{
    NewBinding, NewImport, NewRef, NewScope, NewStringLiteral, NewSymbol, PreciseStatus,
};
use super::utils::{innermost_symbol, unix_now};

/// Row from `(column, value)` pairs; `None` options become `Null` and are not stored.
fn props<const N: usize>(items: [(&str, PropValue); N]) -> Properties {
    items.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The integer primary key bkndb handed back for an auto-increment insert.
fn int_pk(pk: PropValue) -> std::result::Result<i64, BknError> {
    match pk {
        PropValue::Int(v) => Ok(v),
        other => Err(BknError::Encoding(format!(
            "expected integer primary key, got {other:?}"
        ))),
    }
}

impl CacheDb {
    pub fn update_file_metadata(
        &self,
        file_id: i64,
        mtime: Option<i64>,
        size: Option<i64>,
    ) -> Result<()> {
        self.db.write_tx(|b| {
            b.relational().table(self.tables.files.clone()).update(
                &PropValue::Int(file_id),
                |row| {
                    row.insert("mtime".into(), mtime.into());
                    row.insert("size".into(), size.into());
                },
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Invalidate before changing symbol IDs, including if a later file fails.
    pub fn invalidate_precise_generation(&mut self) -> Result<()> {
        let generation = self
            .meta_get("generation")?
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let ids = self
            .all_files()?
            .into_iter()
            .map(|f| f.id)
            .collect::<Vec<_>>();
        self.clear_precise(&ids)?;
        self.meta_set("precise_generation", "invalid")?;
        self.meta_set("generation", &generation.to_string())
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.db.write_tx(|b| {
            b.relational()
                .table(self.tables.meta.clone())
                .upsert(props([("key", key.into()), ("value", value.into())]))?;
            Ok(())
        })?;
        Ok(())
    }

    /// Remove a file and all its analysis rows.
    pub fn delete_file(&self, path: &str) -> Result<()> {
        let t = &self.tables;
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            rel.table(t.embedding_chunks.clone())
                .delete_where_eq("path", &PropValue::from(path))?;
            rel.table(t.meta.clone())
                .delete(&PropValue::from(format!("embedding_file:{path}")))?;
            rel.table(t.meta.clone())
                .delete(&PropValue::from(format!("embedding_excluded:{path}")))?;
            for prefix in ["analyzer:", "diagnostics:", "embedding_count:"] {
                rel.table(t.meta.clone())
                    .delete(&PropValue::from(format!("{prefix}{path}")))?;
            }
            let rows = rel
                .table(t.files.clone())
                .select_eq("path", &PropValue::from(path))?;
            for row in rows {
                let file_id = PropValue::Int(int_pk(row.pk.clone())?);
                super::content::remove(&mut rel, t, int_pk(row.pk.clone())?)?;
                super::purpose::remove(&mut rel, t, int_pk(row.pk.clone())?)?;
                rel.table(t.source_snapshots.clone()).delete(&file_id)?;
                for child in [
                    &t.symbols,
                    &t.imports,
                    &t.refs,
                    &t.scopes,
                    &t.bindings,
                    &t.string_literals,
                ] {
                    rel.table(child.clone())
                        .delete_where_eq("file_id", &file_id)?;
                }
                rel.table(t.files.clone()).delete(&row.pk)?;
            }
            Ok(())
        })?;
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
        let t = &self.tables;
        let now = unix_now();
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            for (ref_id, status, symbol_id, confidence) in results {
                let conf = matches!(status, PreciseStatus::Hit).then_some(*confidence);
                rel.table(t.refs.clone())
                    .update(&PropValue::Int(*ref_id), |row| {
                        row.insert("precise_status".into(), status.as_str().into());
                        row.insert("precise_symbol_id".into(), (*symbol_id).into());
                        row.insert("precise_confidence".into(), conf.into());
                    })?;
            }
            rel.table(t.files.clone())
                .update(&PropValue::Int(file_id), |row| {
                    row.insert("precise_synced_at".into(), now.into());
                })?;
            Ok(())
        })?;
        Ok(())
    }

    /// Forget every language-server answer for the given files, so the next
    /// `--precise` run asks again from scratch.
    pub fn clear_precise(&mut self, file_ids: &[i64]) -> Result<()> {
        let t = &self.tables;
        self.db.write_tx(|b| {
            let mut rel = b.relational();
            for id in file_ids {
                rel.table(t.refs.clone()).update_where_eq(
                    "file_id",
                    &PropValue::Int(*id),
                    &[
                        ("precise_status", PropValue::Null),
                        ("precise_symbol_id", PropValue::Null),
                        ("precise_confidence", PropValue::Null),
                    ],
                )?;
                rel.table(t.files.clone())
                    .update(&PropValue::Int(*id), |row| {
                        row.insert("precise_synced_at".into(), PropValue::Null);
                    })?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Replace all analysis rows for a single file in one transaction.
    ///
    /// `symbols` must be ordered outermost-first so `parent_index` normally
    /// refers to an earlier entry (a forward reference is wired up afterwards).
    /// Each ref is attached to the innermost symbol whose byte range contains
    /// `ref.start_byte`.
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
        source: Option<&[u8]>,
    ) -> Result<()> {
        if let Some(bytes) = source {
            anyhow::ensure!(
                blake3::hash(bytes).to_hex().as_str() == content_hash,
                "source snapshot hash does not match analysis for {path}"
            );
        }
        let prepared = super::content::prepare(path, content_hash, source, symbols)?;
        let t = &self.tables;
        let now = unix_now();

        self.db.write_tx(|b| {
            let mut rel = b.relational();

            let old_chunks = rel
                .table(t.embedding_chunks.clone())
                .select_eq("path", &PropValue::from(path))?;
            for chunk in old_chunks {
                rel.table(t.embedding_chunks.clone())
                    .update(&chunk.pk, |v| {
                        v.insert("valid".into(), false.into());
                    })?;
            }
            rel.table(t.meta.clone())
                .delete(&PropValue::from(format!("embedding_file:{path}")))?;
            // Upsert the file row, keeping its id stable across re-syncs.
            let existing = rel
                .table(t.files.clone())
                .select_eq("path", &PropValue::from(path))?;
            let file_id = match existing.first() {
                Some(row) => {
                    rel.table(t.files.clone()).update(&row.pk, |v| {
                        v.insert("language".into(), language.into());
                        v.insert("content_hash".into(), content_hash.into());
                        v.insert("mtime".into(), mtime.into());
                        v.insert("size".into(), size.into());
                        v.insert("parsed_ok".into(), parsed_ok.into());
                        v.insert("updated_at".into(), now.into());
                        // the file changed, so any language-server answers for it are
                        // stale and must be asked again on the next --precise run
                        v.insert("precise_synced_at".into(), PropValue::Null);
                    })?;
                    int_pk(row.pk.clone())?
                }
                None => int_pk(rel.table(t.files.clone()).insert(props([
                    ("path", path.into()),
                    ("language", language.into()),
                    ("content_hash", content_hash.into()),
                    ("mtime", mtime.into()),
                    ("size", size.into()),
                    ("parsed_ok", parsed_ok.into()),
                    ("updated_at", now.into()),
                ]))?)?,
            };

            // No foreign keys: clear this file's child rows by hand.
            let fid = PropValue::Int(file_id);
            prepared.write(&mut rel, t, file_id, content_hash)?;
            if let Some(bytes) = source {
                let _snapshot_timer = crate::operation::time("source_snapshot_fill");
                rel.table(t.source_snapshots.clone())
                    .upsert(super::snapshots::properties(file_id, content_hash, bytes))?;
            } else {
                rel.table(t.source_snapshots.clone()).delete(&fid)?;
            }
            for child in [
                &t.symbols,
                &t.imports,
                &t.refs,
                &t.scopes,
                &t.bindings,
                &t.string_literals,
            ] {
                rel.table(child.clone()).delete_where_eq("file_id", &fid)?;
            }

            // Symbols: the parent is wired at insert time when it comes earlier.
            let mut ids: Vec<i64> = Vec::with_capacity(symbols.len());
            let mut late_parents: Vec<(usize, usize)> = Vec::new();
            for (i, s) in symbols.iter().enumerate() {
                let parent = s.parent_index.and_then(|p| ids.get(p).copied());
                if let Some(p) = s.parent_index
                    && parent.is_none()
                    && p < symbols.len()
                {
                    late_parents.push((i, p));
                }
                let pk = rel.table(t.symbols.clone()).insert(props([
                    ("file_id", file_id.into()),
                    ("name", s.name.as_str().into()),
                    ("name_lower", s.name.to_ascii_lowercase().into()),
                    ("kind", s.kind.as_str().into()),
                    ("parent_symbol_id", parent.into()),
                    ("is_exported", s.is_exported.into()),
                    ("start_line", s.start_line.into()),
                    ("end_line", s.end_line.into()),
                    ("start_byte", s.start_byte.into()),
                    ("end_byte", s.end_byte.into()),
                    ("signature", s.signature.clone().into()),
                    ("param_count", s.param_count.into()),
                    ("type_name", s.type_name.clone().into()),
                ]))?;
                ids.push(int_pk(pk)?);
            }
            for (i, p) in late_parents {
                let parent_id = ids[p];
                rel.table(t.symbols.clone())
                    .update(&PropValue::Int(ids[i]), |row| {
                        row.insert("parent_symbol_id".into(), parent_id.into());
                    })?;
            }

            // Scopes, same shape as symbols.
            let mut scope_ids: Vec<i64> = Vec::with_capacity(scopes.len());
            let mut late_scopes: Vec<(usize, usize)> = Vec::new();
            for (i, sc) in scopes.iter().enumerate() {
                let parent = sc.parent_index.and_then(|p| scope_ids.get(p).copied());
                if let Some(p) = sc.parent_index
                    && parent.is_none()
                    && p < scopes.len()
                {
                    late_scopes.push((i, p));
                }
                let owner = sc.owner_symbol_index.and_then(|i| ids.get(i).copied());
                let pk = rel.table(t.scopes.clone()).insert(props([
                    ("file_id", file_id.into()),
                    ("parent_scope_id", parent.into()),
                    ("owner_symbol_id", owner.into()),
                    ("kind", sc.kind.as_str().into()),
                    ("start_byte", sc.start_byte.into()),
                    ("end_byte", sc.end_byte.into()),
                ]))?;
                scope_ids.push(int_pk(pk)?);
            }
            for (i, p) in late_scopes {
                let parent_id = scope_ids[p];
                rel.table(t.scopes.clone())
                    .update(&PropValue::Int(scope_ids[i]), |row| {
                        row.insert("parent_scope_id".into(), parent_id.into());
                    })?;
            }

            // Imports (before bindings so `namespace` bindings can point at them).
            let import_ids: Vec<i64> = if imports.is_empty() {
                Vec::new()
            } else {
                let pks = rel
                    .table(t.imports.clone())
                    .insert_bulk(imports.iter().map(|im| {
                        props([
                            ("file_id", file_id.into()),
                            ("raw_specifier", im.raw_specifier.as_str().into()),
                            ("imported_name", im.imported_name.clone().into()),
                            ("alias", im.alias.clone().into()),
                            ("is_relative", im.is_relative.into()),
                            ("start_line", im.start_line.into()),
                        ])
                    }))?;
                pks.into_iter()
                    .map(int_pk)
                    .collect::<std::result::Result<_, _>>()?
            };

            // Bindings.
            let binding_rows: Vec<Properties> = bindings
                .iter()
                .filter_map(|bd| {
                    let scope_id = *scope_ids.get(bd.scope_index)?;
                    let symbol_id = bd.symbol_index.and_then(|i| ids.get(i).copied());
                    let import_id = bd.import_index.and_then(|i| import_ids.get(i).copied());
                    Some(props([
                        ("file_id", file_id.into()),
                        ("scope_id", scope_id.into()),
                        ("name", bd.name.as_str().into()),
                        ("binding_kind", bd.binding_kind.as_str().into()),
                        ("symbol_id", symbol_id.into()),
                        ("import_id", import_id.into()),
                        ("type_expr", bd.type_expr.clone().into()),
                    ]))
                })
                .collect();
            if !binding_rows.is_empty() {
                rel.table(t.bindings.clone()).insert_bulk(binding_rows)?;
            }

            // Refs, attached to their enclosing symbol. `resolved_local_symbol_index`
            // (from the analyzer's scope walk) becomes a same-file `resolved_symbol_id`.
            let ref_rows: Vec<Properties> = refs
                .iter()
                .map(|rf| {
                    let enclosing = innermost_symbol(symbols, &ids, rf.start_byte);
                    let resolved = rf
                        .resolved_local_symbol_index
                        .and_then(|i| ids.get(i).copied());
                    let resolved_conf: Option<f64> = resolved.map(|_| 0.95);
                    props([
                        ("file_id", file_id.into()),
                        ("from_symbol_id", enclosing.into()),
                        ("name", rf.name.as_str().into()),
                        ("ref_kind", rf.ref_kind.as_str().into()),
                        ("receiver", rf.receiver.clone().into()),
                        ("start_line", rf.start_line.into()),
                        ("arg_count", rf.arg_count.into()),
                        ("receiver_kind", rf.receiver_kind.as_str().into()),
                        ("local_only", rf.local_only.into()),
                        ("resolved_symbol_id", resolved.into()),
                        ("resolved_confidence", resolved_conf.into()),
                        ("name_start_byte", rf.name_start_byte.into()),
                    ])
                })
                .collect();
            if !ref_rows.is_empty() {
                rel.table(t.refs.clone()).insert_bulk(ref_rows)?;
            }

            // String literals.
            if !string_literals.is_empty() {
                rel.table(t.string_literals.clone())
                    .insert_bulk(string_literals.iter().map(|sl| {
                        props([
                            ("file_id", file_id.into()),
                            ("value", sl.value.as_str().into()),
                            ("callee", sl.callee.clone().into()),
                            ("line", sl.line.into()),
                        ])
                    }))?;
            }

            #[cfg(test)]
            if super::snapshots::FAIL_ANALYSIS_COMMIT.with(|flag| flag.replace(false)) {
                return Err(BknError::Encoding(
                    "injected analysis commit failure".into(),
                ));
            }

            Ok(())
        })?;
        Ok(())
    }
}
