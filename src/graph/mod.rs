//! In-memory relation graph built from the cache, plus renderers.

pub mod community;
pub mod query;
pub mod render;
pub mod resolve;

use serde::Serialize;

/// A resolved code relation graph ready to render.
#[derive(Debug, Serialize)]
pub struct CodeGraph {
    /// Per-reference verification coverage; selective completion is not full verification.
    pub precision: serde_json::Value,
    pub resolution_summary: ResolutionSummary,
    pub version: u32,
    pub root: String,
    pub generated_at: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Subsystems detected over file-level import/call links; nodes refer to them by `community`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub communities: Vec<CommunityInfo>,
}

/// A detected subsystem, as shipped with the graph.
#[derive(Debug, Clone, Serialize)]
pub struct CommunityInfo {
    pub id: u32,
    pub label: String,
    /// Number of files in the community.
    pub size: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<crate::analysis::semantic::SymbolFacts>,
    pub id: String,
    /// dir | file | function | method | struct | enum | trait | type | variable | module | impl | macro | external
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Parent directory of `path` (project-relative), or `None` at the repo root.
    /// Lets the viewer group and roll a large graph up to a folder-level view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<[i64; 2]>,
    /// Count of incident relation edges (imports/calls/references, not `contains`).
    /// Used client-side to size hubs and to pick which symbols to drop first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<u32>,
    /// Id into `CodeGraph::communities`; symbols inherit their file's community.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<crate::graph::resolve::refs::Resolution>,
    pub source: String,
    pub target: String,
    /// imports | calls | references | contains
    pub kind: String,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
}

/// Stable node id for a file.
pub fn file_id(rel_path: &str) -> String {
    format!("file:{rel_path}")
}

/// Stable node id for a directory roll-up node.
pub fn dir_id(rel_dir: &str) -> String {
    format!("dir:{rel_dir}")
}

/// A declaration identity includes byte range and kind, including same-line overloads.
pub fn declaration_id(path: &str, name: &str, start: i64, end: i64, kind: &str) -> String {
    format!("sym:{path}#{name}@{start}:{end}:{kind}")
}

/// Stable node id for an unresolved external module.
pub fn external_id(specifier: &str) -> String {
    format!("ext:{specifier}")
}

/// Counts cover references in the requested path/edge-kind scope, before node caps.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ResolutionSummary {
    pub eligible: usize,
    pub resolved: usize,
    pub ambiguous: usize,
    pub unresolved: usize,
    pub local_only: usize,
    pub external: usize,
    pub below_min_confidence: usize,
}
