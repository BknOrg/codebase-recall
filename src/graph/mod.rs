//! In-memory relation graph built from the cache, plus renderers.

pub mod render;
pub mod resolve;

use serde::Serialize;

/// A resolved code relation graph ready to render.
#[derive(Debug, Serialize)]
pub struct CodeGraph {
    pub version: u32,
    pub root: String,
    pub generated_at: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    /// file | function | method | struct | enum | trait | type | variable | module | impl | macro | external
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<[i64; 2]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    /// imports | calls | references | contains
    pub kind: String,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
}

/// Stable node id for a file.
pub fn file_id(rel_path: &str) -> String {
    format!("file:{rel_path}")
}

/// Stable node id for a symbol.
pub fn symbol_id(rel_path: &str, name: &str, start_line: i64) -> String {
    format!("sym:{rel_path}#{name}@{start_line}")
}

/// Stable node id for an unresolved external module.
pub fn external_id(specifier: &str) -> String {
    format!("ext:{specifier}")
}
