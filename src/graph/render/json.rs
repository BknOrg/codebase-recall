use anyhow::Result;

use crate::graph::CodeGraph;

/// Pretty-printed JSON. Schema is stable at `version: 1`:
/// `{ version, root, generated_at, nodes[], edges[] }`.
pub fn render(graph: &CodeGraph) -> Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}
