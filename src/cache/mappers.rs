use rusqlite::Row;

use crate::cache::models::{
    BindingRow, FileRow, ImportRow, RefRow, ScopeRow, StringLiteralRow, SymbolRow,
};

pub const REF_COLUMNS: &str = "SELECT id, file_id, from_symbol_id, name, ref_kind, receiver, start_line,
            arg_count, receiver_kind, local_only, resolved_symbol_id, resolved_confidence,
            name_start_byte, precise_symbol_id, precise_confidence, precise_status";

pub fn map_ref_row(r: &Row) -> rusqlite::Result<RefRow> {
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
        name_start_byte: r.get(12)?,
        precise_symbol_id: r.get(13)?,
        precise_confidence: r.get(14)?,
        precise_status: r.get(15)?,
    })
}

pub fn map_file_row(r: &Row) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        path: r.get(1)?,
        language: r.get(2)?,
        content_hash: r.get(3)?,
        mtime: r.get(4)?,
        size: r.get(5)?,
        parsed_ok: r.get::<_, i64>(6)? != 0,
        precise_synced_at: r.get(7)?,
    })
}

pub fn map_symbol_row(r: &Row) -> rusqlite::Result<SymbolRow> {
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
}

pub fn map_import_row(r: &Row) -> rusqlite::Result<ImportRow> {
    Ok(ImportRow {
        id: r.get(0)?,
        file_id: r.get(1)?,
        raw_specifier: r.get(2)?,
        imported_name: r.get(3)?,
        alias: r.get(4)?,
        is_relative: r.get::<_, i64>(5)? != 0,
        start_line: r.get(6)?,
    })
}

pub fn map_scope_row(r: &Row) -> rusqlite::Result<ScopeRow> {
    Ok(ScopeRow {
        id: r.get(0)?,
        file_id: r.get(1)?,
        parent_scope_id: r.get(2)?,
        owner_symbol_id: r.get(3)?,
        kind: r.get(4)?,
        start_byte: r.get(5)?,
        end_byte: r.get(6)?,
    })
}

pub fn map_binding_row(r: &Row) -> rusqlite::Result<BindingRow> {
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
}

pub fn map_string_literal_row(r: &Row) -> rusqlite::Result<StringLiteralRow> {
    Ok(StringLiteralRow {
        id: r.get(0)?,
        file_id: r.get(1)?,
        value: r.get(2)?,
        callee: r.get(3)?,
        line: r.get(4)?,
    })
}
