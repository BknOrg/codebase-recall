use anyhow::Result;

use crate::graph::CodeGraph;

/// Pretty-printed JSON. Schema `version: 3`:
/// `{ version, root, generated_at, nodes[], edges[] }`.
/// v3 adds typed semantic facts, declaration identities and resolution evidence.
/// v2 adds `nodes[].dir` / `nodes[].degree` and `dir:` roll-up nodes with
/// `contains` edges down to files.
pub fn render(graph: &CodeGraph) -> Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}
