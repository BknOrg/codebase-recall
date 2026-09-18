use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct DigestReport {
    pub project: String,
    pub total_files: usize,
    pub total_symbols: usize,
    pub public_symbols: usize,
    pub languages: HashMap<String, usize>,
    pub hubs: Vec<HubItem>,
    pub modules: Vec<ModuleDigest>,
}

#[derive(Debug, Serialize)]
pub struct HubItem {
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub degree: u32,
}

#[derive(Debug, Serialize)]
pub struct ModuleDigest {
    pub directory: String,
    pub files: Vec<FileDigest>,
}

#[derive(Debug, Serialize)]
pub struct FileDigest {
    pub path: String,
    pub language: String,
    pub types: Vec<TypeDigest>,
    pub functions: Vec<SymbolItem>,
}

#[derive(Debug, Serialize)]
pub struct TypeDigest {
    pub name: String,
    pub kind: String,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub has_diagram: bool,
    pub has_diagram_in_fields: bool,
    pub methods: Vec<SymbolItem>,
}

#[derive(Debug, Serialize)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub line: Option<i64>,
    pub is_exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub has_diagram: bool,
}
