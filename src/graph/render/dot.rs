use crate::graph::{CodeGraph, Node};
use std::collections::BTreeMap;

/// Graphviz DOT. Symbols are grouped into a `subgraph cluster_*` per file.
pub fn render(graph: &CodeGraph) -> String {
    let mut out = String::from("digraph code {\n");
    out.push_str("  graph [rankdir=LR, fontname=\"sans-serif\", compound=true];\n");
    out.push_str("  node  [fontname=\"sans-serif\", fontsize=10, style=filled, fillcolor=\"#f5f5f5\"];\n");
    out.push_str("  edge  [fontname=\"sans-serif\", fontsize=8, color=\"#888888\"];\n\n");

    // Group symbol nodes by their file path; file/external nodes stand alone.
    let mut by_file: BTreeMap<&str, Vec<&Node>> = BTreeMap::new();
    let mut loose: Vec<&Node> = Vec::new();
    for n in &graph.nodes {
        match (&n.path, n.kind.as_str()) {
            (Some(p), k) if k != "file" => by_file.entry(p.as_str()).or_default().push(n),
            _ => loose.push(n),
        }
    }

    for n in &loose {
        out.push_str(&format!(
            "  {} [label={}, shape={}];\n",
            quote(&n.id),
            quote(&n.label),
            shape(&n.kind)
        ));
    }

    for (i, (path, syms)) in by_file.iter().enumerate() {
        out.push_str(&format!("  subgraph cluster_{i} {{\n"));
        out.push_str(&format!("    label={};\n", quote(path)));
        out.push_str("    style=rounded; color=\"#cccccc\";\n");
        for n in syms {
            out.push_str(&format!(
                "    {} [label={}, shape={}];\n",
                quote(&n.id),
                quote(&n.label),
                shape(&n.kind)
            ));
        }
        out.push_str("  }\n");
    }

    out.push('\n');
    for e in &graph.edges {
        out.push_str(&format!(
            "  {} -> {} [style={}, tooltip={}];\n",
            quote(&e.source),
            quote(&e.target),
            edge_style(&e.kind),
            quote(&format!("{} ({:.2})", e.kind, e.confidence)),
        ));
    }

    out.push_str("}\n");
    out
}

fn shape(kind: &str) -> &'static str {
    match kind {
        "file" => "box",
        "struct" | "enum" | "trait" | "interface" | "type" | "class" => "diamond",
        "variable" => "note",
        "external" => "box3d",
        _ => "ellipse",
    }
}

fn edge_style(kind: &str) -> &'static str {
    match kind {
        "imports" => "dashed",
        "references" => "dotted",
        "contains" => "solid",
        _ => "solid",
    }
}

fn quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
