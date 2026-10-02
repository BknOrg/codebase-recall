//! Decode bkndb rows into the plain `*Row` structs in [`super::models`].
//!
//! bkndb keeps the primary key in `Row::pk` and omits `Null` columns, so every
//! getter treats a missing key as `None`.

use bkndb::relational::Row;
use bkndb::value::{PropValue, Properties};

use crate::cache::models::{
    BindingRow, FileRow, ImportRow, RefRow, ScopeRow, StringLiteralRow, SymbolRow,
};

fn int(p: &Properties, k: &str) -> Option<i64> {
    match p.get(k) {
        Some(PropValue::Int(v)) => Some(*v),
        _ => None,
    }
}

fn float(p: &Properties, k: &str) -> Option<f64> {
    match p.get(k) {
        Some(PropValue::Float(v)) => Some(*v),
        _ => None,
    }
}

fn text(p: &Properties, k: &str) -> Option<String> {
    match p.get(k) {
        Some(PropValue::Str(v)) => Some(v.clone()),
        _ => None,
    }
}

fn flag(p: &Properties, k: &str) -> bool {
    matches!(p.get(k), Some(PropValue::Bool(true)))
}

/// The auto-increment `id`, which lives in the row key rather than the values.
pub fn id(r: &Row) -> i64 {
    match r.pk {
        PropValue::Int(v) => v,
        _ => 0,
    }
}

pub fn file_row(r: &Row) -> FileRow {
    let p = &r.values;
    FileRow {
        id: id(r),
        path: text(p, "path").unwrap_or_default(),
        language: text(p, "language").unwrap_or_default(),
        content_hash: text(p, "content_hash").unwrap_or_default(),
        mtime: int(p, "mtime"),
        size: int(p, "size"),
        parsed_ok: flag(p, "parsed_ok"),
        precise_synced_at: int(p, "precise_synced_at"),
    }
}

pub fn symbol_row(r: &Row) -> SymbolRow {
    let p = &r.values;
    SymbolRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        name: text(p, "name").unwrap_or_default(),
        kind: text(p, "kind").unwrap_or_default(),
        parent_symbol_id: int(p, "parent_symbol_id"),
        is_exported: flag(p, "is_exported"),
        start_line: int(p, "start_line"),
        end_line: int(p, "end_line"),
        start_byte: int(p, "start_byte"),
        end_byte: int(p, "end_byte"),
        signature: text(p, "signature"),
        param_count: int(p, "param_count"),
        type_name: text(p, "type_name"),
    }
}

pub fn import_row(r: &Row) -> ImportRow {
    let p = &r.values;
    ImportRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        raw_specifier: text(p, "raw_specifier").unwrap_or_default(),
        imported_name: text(p, "imported_name"),
        alias: text(p, "alias"),
        is_relative: flag(p, "is_relative"),
        start_line: int(p, "start_line"),
    }
}

pub fn ref_row(r: &Row) -> RefRow {
    let p = &r.values;
    RefRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        from_symbol_id: int(p, "from_symbol_id"),
        name: text(p, "name").unwrap_or_default(),
        ref_kind: text(p, "ref_kind").unwrap_or_default(),
        receiver: text(p, "receiver"),
        start_line: int(p, "start_line"),
        arg_count: int(p, "arg_count"),
        receiver_kind: text(p, "receiver_kind"),
        local_only: flag(p, "local_only"),
        resolved_symbol_id: int(p, "resolved_symbol_id"),
        resolved_confidence: float(p, "resolved_confidence"),
        name_start_byte: int(p, "name_start_byte"),
        precise_symbol_id: int(p, "precise_symbol_id"),
        precise_confidence: float(p, "precise_confidence"),
        precise_status: text(p, "precise_status"),
    }
}

pub fn scope_row(r: &Row) -> ScopeRow {
    let p = &r.values;
    ScopeRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        parent_scope_id: int(p, "parent_scope_id"),
        owner_symbol_id: int(p, "owner_symbol_id"),
        kind: text(p, "kind").unwrap_or_default(),
        start_byte: int(p, "start_byte").unwrap_or_default(),
        end_byte: int(p, "end_byte").unwrap_or_default(),
    }
}

pub fn binding_row(r: &Row) -> BindingRow {
    let p = &r.values;
    BindingRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        scope_id: int(p, "scope_id").unwrap_or_default(),
        name: text(p, "name").unwrap_or_default(),
        binding_kind: text(p, "binding_kind").unwrap_or_default(),
        symbol_id: int(p, "symbol_id"),
        import_id: int(p, "import_id"),
        type_expr: text(p, "type_expr"),
    }
}

pub fn string_literal_row(r: &Row) -> StringLiteralRow {
    let p = &r.values;
    StringLiteralRow {
        id: id(r),
        file_id: int(p, "file_id").unwrap_or_default(),
        value: text(p, "value").unwrap_or_default(),
        callee: text(p, "callee"),
        line: int(p, "line"),
    }
}
