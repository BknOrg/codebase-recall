//! Rust extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

use crate::analysis::ParsedFile;
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str) -> ParsedFile {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return ParsedFile::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return ParsedFile::default();
    };

    let mut walker = Walker {
        src: source.as_bytes(),
        out: ParsedFile {
            parse_ok: !tree.root_node().has_error(),
            ..ParsedFile::default()
        },
        stack: Vec::new(),
    };
    walker.walk(tree.root_node());
    walker.out
}

struct Walker<'a> {
    src: &'a [u8],
    out: ParsedFile,
    /// Indices into `out.symbols` for the current lexical parent chain.
    stack: Vec<usize>,
}

impl<'a> Walker<'a> {
    fn walk(&mut self, node: Node) {
        match node.kind() {
            "mod_item" => {
                // `mod foo;` (no inline body) pulls in a sibling file.
                let has_body = node.child_by_field_name("body").is_some()
                    || child_of_kind(node, "declaration_list").is_some();
                if !has_body {
                    if let Some(n) = node.child_by_field_name("name") {
                        let name = self.text(n).to_string();
                        self.out.imports.push(NewImport {
                            raw_specifier: format!("self::{name}"),
                            imported_name: Some(name),
                            alias: None,
                            is_relative: true,
                            start_line: self.line(node),
                        });
                    }
                }
                let idx = self.push_symbol(node);
                if let Some(i) = idx {
                    self.stack.push(i);
                }
                self.walk_children(node);
                if idx.is_some() {
                    self.stack.pop();
                }
            }
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "union_item"
            | "type_item" | "const_item" | "static_item" | "impl_item"
            | "macro_definition" => {
                let idx = self.push_symbol(node);
                if let Some(i) = idx {
                    self.stack.push(i);
                }
                self.walk_children(node);
                if idx.is_some() {
                    self.stack.pop();
                }
            }
            "use_declaration" => self.collect_use(node),
            "call_expression" => {
                self.collect_call(node);
                self.walk_children(node);
            }
            "macro_invocation" => {
                self.collect_macro(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        let children: Vec<Node> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };
        for child in children {
            self.walk(child);
        }
    }

    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.src).unwrap_or("")
    }

    fn line(&self, node: Node) -> i64 {
        node.start_position().row as i64 + 1
    }

    fn named(&self, node: Node) -> Option<String> {
        node.child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())
    }

    fn has_pub(&self, node: Node) -> bool {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .any(|c| c.kind() == "visibility_modifier")
    }

    fn parent_is_impl_or_trait(&self) -> bool {
        self.stack.last().is_some_and(|&i| {
            matches!(self.out.symbols[i].kind.as_str(), "impl" | "trait")
        })
    }

    fn push_symbol(&mut self, node: Node) -> Option<usize> {
        let (name, kind) = match node.kind() {
            "impl_item" => {
                let ty = node.child_by_field_name("type")?;
                let ty_txt = self.text(ty).to_string();
                let name = match node.child_by_field_name("trait") {
                    Some(tr) => format!("{} for {}", self.text(tr), ty_txt),
                    None => ty_txt,
                };
                (name, "impl")
            }
            "function_item" => {
                let n = node.child_by_field_name("name")?;
                let kind = if self.parent_is_impl_or_trait() {
                    "method"
                } else {
                    "function"
                };
                (self.text(n).to_string(), kind)
            }
            "struct_item" | "union_item" => (self.named(node)?, "struct"),
            "enum_item" => (self.named(node)?, "enum"),
            "trait_item" => (self.named(node)?, "trait"),
            "type_item" => (self.named(node)?, "type"),
            "const_item" | "static_item" => (self.named(node)?, "variable"),
            "mod_item" => (self.named(node)?, "module"),
            "macro_definition" => (self.named(node)?, "macro"),
            _ => return None,
        };

        let parent_index = self.stack.last().copied();
        self.out.symbols.push(NewSymbol {
            name,
            kind: kind.to_string(),
            parent_index,
            is_exported: self.has_pub(node),
            start_line: node.start_position().row as i64 + 1,
            end_line: node.end_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
            end_byte: node.end_byte() as i64,
            signature: None,
        });
        Some(self.out.symbols.len() - 1)
    }

    fn collect_use(&mut self, node: Node) {
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let line = node.start_position().row as i64 + 1;
        let mut paths: Vec<UsePath> = Vec::new();
        self.flatten_use(arg, "", &mut paths);

        for UsePath {
            path,
            alias,
            wildcard,
        } in paths
        {
            if path.is_empty() {
                continue;
            }
            let is_relative = ["self", "super", "crate"]
                .iter()
                .any(|p| path == *p || path.starts_with(&format!("{p}::")));
            let imported_name = if wildcard {
                None
            } else {
                path.rsplit("::").next().map(str::to_string)
            };
            self.out.imports.push(NewImport {
                raw_specifier: path,
                imported_name,
                alias,
                is_relative,
                start_line: line,
            });
        }
    }

    fn flatten_use(&self, node: Node, prefix: &str, out: &mut Vec<UsePath>) {
        match node.kind() {
            "identifier" | "scoped_identifier" | "type_identifier" | "primitive_type"
            | "self" | "super" | "crate" | "metavariable" => {
                out.push(UsePath::plain(join(prefix, self.text(node))));
            }
            "use_as_clause" => {
                let path = node
                    .child_by_field_name("path")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let alias = node
                    .child_by_field_name("alias")
                    .map(|n| self.text(n).to_string());
                out.push(UsePath {
                    path: join(prefix, path),
                    alias,
                    wildcard: false,
                });
            }
            "use_wildcard" => {
                let inner = node
                    .named_child(0)
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let full = if inner.is_empty() {
                    prefix.trim_end_matches("::").to_string()
                } else {
                    join(prefix, inner)
                };
                out.push(UsePath {
                    path: full,
                    alias: None,
                    wildcard: true,
                });
            }
            "scoped_use_list" => {
                let base = node
                    .child_by_field_name("path")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let new_prefix = join(prefix, base);
                if let Some(list) = node.child_by_field_name("list") {
                    let mut cursor = list.walk();
                    for child in list.named_children(&mut cursor) {
                        self.flatten_use(child, &new_prefix, out);
                    }
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.flatten_use(child, prefix, out);
                }
            }
            _ => {
                let text = self.text(node);
                if !text.is_empty() {
                    out.push(UsePath::plain(join(prefix, text)));
                }
            }
        }
    }

    fn collect_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let (name, receiver) = self.callee_name(func);
        if name.is_empty() {
            return;
        }
        self.out.refs.push(NewRef {
            name,
            ref_kind: "call".to_string(),
            receiver,
            start_line: node.start_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
        });
    }

    fn callee_name(&self, func: Node) -> (String, Option<String>) {
        match func.kind() {
            "identifier" => (self.text(func).to_string(), None),
            "field_expression" => {
                let field = func
                    .child_by_field_name("field")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let receiver = func
                    .child_by_field_name("value")
                    .map(|n| self.text(n).to_string());
                (field.to_string(), receiver)
            }
            "scoped_identifier" => {
                let name = func
                    .child_by_field_name("name")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let path = func
                    .child_by_field_name("path")
                    .map(|n| self.text(n).to_string());
                (name.to_string(), path)
            }
            "generic_function" => func
                .child_by_field_name("function")
                .map(|inner| self.callee_name(inner))
                .unwrap_or_default(),
            _ => (String::new(), None),
        }
    }

    fn collect_macro(&mut self, node: Node) {
        let Some(m) = node.child_by_field_name("macro") else {
            return;
        };
        let name = self.text(m);
        if name.is_empty() {
            return;
        }
        self.out.refs.push(NewRef {
            name: format!("{name}!"),
            ref_kind: "call".to_string(),
            receiver: None,
            start_line: node.start_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
        });
    }
}

struct UsePath {
    path: String,
    alias: Option<String>,
    wildcard: bool,
}

impl UsePath {
    fn plain(path: String) -> Self {
        Self {
            path,
            alias: None,
            wildcard: false,
        }
    }
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

fn join(prefix: &str, seg: &str) -> String {
    match (prefix.is_empty(), seg.is_empty()) {
        (true, _) => seg.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}::{seg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_functions_structs_and_methods() {
        let src = r#"
pub struct Widget { size: u32 }

impl Widget {
    pub fn new() -> Self { Widget { size: 0 } }
    fn grow(&mut self) { self.size += 1; }
}

fn helper() {}

pub fn run() {
    let w = Widget::new();
    helper();
}
"#;
        let p = parse(src);
        assert!(p.parse_ok);

        let by_name = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(by_name("Widget").unwrap().kind, "struct");
        assert!(by_name("Widget").unwrap().is_exported);
        assert_eq!(by_name("new").unwrap().kind, "method");
        assert_eq!(by_name("grow").unwrap().kind, "method");
        assert_eq!(by_name("helper").unwrap().kind, "function");
        assert!(!by_name("helper").unwrap().is_exported);
        assert_eq!(by_name("run").unwrap().kind, "function");

        // `new` is nested under the `impl Widget` symbol.
        let impl_idx = p.symbols.iter().position(|s| s.kind == "impl").unwrap();
        assert_eq!(by_name("new").unwrap().parent_index, Some(impl_idx));

        let call_names: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(call_names.contains(&"new")); // Widget::new()
        assert!(call_names.contains(&"helper"));
    }

    #[test]
    fn flattens_use_declarations() {
        let src = r#"
use std::collections::{HashMap, HashSet};
use crate::cache::CacheDb;
use super::walker as w;
use anyhow::*;
"#;
        let p = parse(src);
        let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);

        assert!(spec("std::collections::HashMap").is_some());
        assert!(spec("std::collections::HashSet").is_some());
        let cc = spec("crate::cache::CacheDb").unwrap();
        assert!(cc.is_relative);
        assert_eq!(cc.imported_name.as_deref(), Some("CacheDb"));
        let sup = spec("super::walker").unwrap();
        assert_eq!(sup.alias.as_deref(), Some("w"));
        assert!(p.imports.iter().any(|i| i.raw_specifier == "anyhow"));
    }
}
