//! AST body provenance for the experimental compact embedding representation.
use super::Language;
use std::{collections::BTreeMap, path::Path};
pub type Bodies = BTreeMap<(usize, usize), Vec<(usize, usize)>>;
pub fn parse(path: &str, source: &str) -> Option<Bodies> {
    let tree = parse_tree(path, source)?;
    Some(bodies(&tree))
}
pub(crate) fn parse_tree(path: &str, source: &str) -> Option<tree_sitter::Tree> {
    let language = Language::from_path(Path::new(path))?;
    let grammar: tree_sitter::Language = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).ok()?;
    let mut tree = parser.parse(source, None)?;
    if tree.root_node().has_error() && language == Language::Kotlin {
        tree = parser.parse(super::kotlin_comments::mask(source), None)?;
    }
    if tree.root_node().has_error() {
        return None;
    }
    Some(tree)
}
pub(crate) fn bodies(tree: &tree_sitter::Tree) -> Bodies {
    let mut bodies: Bodies = BTreeMap::new();
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        let body = node
            .child_by_field_name("body")
            .or_else(|| node.child_by_field_name("block"))
            .or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor).find(|c| {
                    matches!(
                        c.kind(),
                        "block"
                            | "function_body"
                            | "class_body"
                            | "interface_body"
                            | "statement_block"
                            | "declaration_list"
                    )
                })
            });
        let entry = bodies
            .entry((node.start_byte(), node.end_byte()))
            .or_default();
        if let Some(body) = body {
            let span = (body.start_byte(), body.end_byte());
            if !entry.contains(&span) {
                entry.push(span);
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    bodies
}
