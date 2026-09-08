//! Python extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

use crate::analysis::scope::{self, ScopeStack};
use crate::analysis::ParsedFile;
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str) -> ParsedFile {
    let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
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
        sc: ScopeStack::new(source.len() as i64),
    };
    walker.walk(tree.root_node());
    walker.sc.finish_into(&mut walker.out);
    scope::resolve_locals(&mut walker.out);
    walker.out
}

struct Walker<'a> {
    src: &'a [u8],
    out: ParsedFile,
    stack: Vec<usize>,
    sc: ScopeStack,
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.src).unwrap_or("")
    }

    fn line(&self, node: Node) -> i64 {
        node.start_position().row as i64 + 1
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => {
                let kind = if self.parent_is_class() {
                    "method"
                } else {
                    "function"
                };
                self.enter_symbol(node, kind);
            }
            "class_definition" => self.enter_symbol(node, "class"),
            "import_statement" => self.collect_import(node, false),
            "import_from_statement" => self.collect_import(node, true),
            "assignment" => {
                self.collect_assignment(node);
                self.walk_children(node);
            }
            "call" => {
                self.collect_call(node);
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

    fn parent_is_class(&self) -> bool {
        self.stack
            .last()
            .is_some_and(|&i| self.out.symbols[i].kind == "class")
    }

    fn enter_symbol(&mut self, node: Node, kind: &str) {
        let idx = self.push_symbol(node, kind);
        if let Some(i) = idx {
            let (name, real_kind) = {
                let s = &self.out.symbols[i];
                (s.name.clone(), s.kind.clone())
            };
            let scope_kind = match real_kind.as_str() {
                "class" => "class",
                "method" => "method",
                _ => "function",
            };
            if real_kind != "method" {
                self.sc.bind(&name, "symbol", Some(i), None, None);
            }
            self.stack.push(i);
            self.sc
                .push(scope_kind, Some(i), node.start_byte() as i64, node.end_byte() as i64);

            match real_kind.as_str() {
                "function" | "method" => self.bind_params(node),
                "class" => self.bind_class_fields(node),
                _ => {}
            }

            self.walk_children(node);

            self.sc.pop();
            self.stack.pop();
        } else {
            self.walk_children(node);
        }
    }

    fn bind_params(&mut self, func: Node) {
        let Some(params) = func.child_by_field_name("parameters") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = params.walk();
            params.named_children(&mut c).collect()
        };
        for p in kids {
            let (name, ty) = match p.kind() {
                "identifier" => (self.text(p).to_string(), None),
                "typed_parameter" | "typed_default_parameter" => {
                    let n = p
                        .named_child(0)
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_default();
                    let t = p
                        .child_by_field_name("type")
                        .map(|x| simple_type_name(self.text(x)));
                    (n, t)
                }
                "default_parameter" => (
                    p.child_by_field_name("name")
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_default(),
                    None,
                ),
                _ => continue,
            };
            self.sc.bind(&name, "param", None, None, ty);
        }
    }

    /// Class-body `name: T` annotations (incl. dataclass fields) become field bindings.
    fn bind_class_fields(&mut self, class: Node) {
        let Some(body) = class.child_by_field_name("body") else {
            return;
        };
        let stmts: Vec<Node> = {
            let mut c = body.walk();
            body.named_children(&mut c).collect()
        };
        for s in stmts {
            let inner = if s.kind() == "expression_statement" {
                s.named_child(0)
            } else {
                Some(s)
            };
            let Some(assign) = inner.filter(|n| n.kind() == "assignment") else {
                continue;
            };
            let (Some(lhs), Some(ty)) = (
                assign.child_by_field_name("left"),
                assign.child_by_field_name("type"),
            ) else {
                continue;
            };
            if lhs.kind() != "identifier" {
                continue;
            }
            let name = self.text(lhs).to_string();
            self.sc
                .bind(&name, "field", None, None, Some(simple_type_name(self.text(ty))));
        }
    }

    fn param_count(&self, node: Node) -> Option<i64> {
        let params = node.child_by_field_name("parameters")?;
        let mut c = params.walk();
        Some(params.named_children(&mut c).count() as i64)
    }

    fn enclosing_class_name(&self) -> Option<String> {
        self.stack
            .iter()
            .rev()
            .find_map(|&i| (self.out.symbols[i].kind == "class").then(|| self.out.symbols[i].name.clone()))
    }

    fn push_symbol(&mut self, node: Node, kind: &str) -> Option<usize> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())?;
        let is_exported = !name.starts_with('_');

        let (param_count, type_name) = if matches!(kind, "function" | "method") {
            (
                self.param_count(node),
                self.enclosing_class_name().filter(|_| kind == "method"),
            )
        } else {
            (None, None)
        };

        self.out.symbols.push(NewSymbol {
            name,
            kind: kind.to_string(),
            parent_index: self.stack.last().copied(),
            is_exported,
            start_line: self.line(node),
            end_line: node.end_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
            end_byte: node.end_byte() as i64,
            signature: None,
            param_count,
            type_name,
        });
        Some(self.out.symbols.len() - 1)
    }

    /// Module-level or class-level `name = ...` bindings become variable symbols.
    fn collect_assignment(&mut self, node: Node) {
        let at_module = self.stack.is_empty();
        let in_class = self.parent_is_class();
        if !at_module && !in_class {
            return;
        }
        let Some(lhs) = node.child_by_field_name("left") else {
            return;
        };
        if lhs.kind() != "identifier" {
            return; // skip tuple/attribute targets
        }
        let name = self.text(lhs).to_string();
        if name.is_empty() {
            return;
        }
        self.out.symbols.push(NewSymbol {
            name: name.clone(),
            kind: "variable".to_string(),
            parent_index: self.stack.last().copied(),
            is_exported: !name.starts_with('_'),
            start_line: self.line(node),
            end_line: node.end_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
            end_byte: node.end_byte() as i64,
            signature: None,
            param_count: None,
            type_name: None,
        });
        let idx = self.out.symbols.len() - 1;
        if at_module {
            self.sc.bind(&name, "symbol", Some(idx), None, None);
        }
    }

    fn collect_import(&mut self, node: Node, is_from: bool) {
        let line = self.line(node);

        if !is_from {
            // `import a.b.c`, `import a.b as x`
            let children: Vec<Node> = {
                let mut c = node.walk();
                node.named_children(&mut c).collect()
            };
            for child in children {
                match child.kind() {
                    "dotted_name" => {
                        let path = self.text(child).to_string();
                        let bound = path.split('.').next().unwrap_or(&path).to_string();
                        self.push_import(path, Some(&bound), None, false, line, Some("namespace"));
                    }
                    "aliased_import" => {
                        let path = child
                            .child_by_field_name("name")
                            .map(|n| self.text(n).to_string())
                            .unwrap_or_default();
                        let alias = child
                            .child_by_field_name("alias")
                            .map(|n| self.text(n).to_string());
                        let bound = alias
                            .clone()
                            .unwrap_or_else(|| path.split('.').next().unwrap_or("").to_string());
                        self.push_import(path, Some(&bound), alias, false, line, Some("namespace"));
                    }
                    _ => {}
                }
            }
            return;
        }

        // `from X import a, b as c` / `from . import x` / `from .pkg import *`
        let module_node = node.child_by_field_name("module_name");
        let module = module_node.map(|n| self.text(n).to_string()).unwrap_or_default();
        let is_relative = module.starts_with('.')
            || module_node.is_some_and(|n| n.kind() == "relative_import");

        let names: Vec<Node> = {
            let mut c = node.walk();
            node.named_children(&mut c).collect()
        };
        let mut any = false;
        for child in names {
            if Some(child) == module_node {
                continue;
            }
            match child.kind() {
                "dotted_name" | "identifier" => {
                    let n = self.text(child).to_string();
                    self.push_import(module.clone(), Some(&n), None, is_relative, line, Some("import"));
                    any = true;
                }
                "aliased_import" => {
                    let n = child
                        .child_by_field_name("name")
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_default();
                    let alias = child
                        .child_by_field_name("alias")
                        .map(|x| self.text(x).to_string());
                    self.push_import(module.clone(), Some(&n), alias, is_relative, line, Some("import"));
                    any = true;
                }
                "wildcard_import" => {
                    self.push_import(module.clone(), None, None, is_relative, line, None);
                    any = true;
                }
                _ => {}
            }
        }
        if !any {
            self.push_import(module, None, None, is_relative, line, None);
        }
    }

    fn push_import(
        &mut self,
        specifier: String,
        imported_name: Option<&str>,
        alias: Option<String>,
        is_relative: bool,
        line: i64,
        bind_kind: Option<&str>,
    ) {
        let import_index = self.out.imports.len();
        if let Some(kind) = bind_kind {
            if let Some(local) = alias.as_deref().or(imported_name) {
                self.sc.bind(local, kind, None, Some(import_index), None);
            }
        }
        self.out.imports.push(NewImport {
            raw_specifier: specifier,
            imported_name: imported_name.map(str::to_string),
            alias,
            is_relative,
            start_line: line,
        });
    }

    fn collect_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let (name, receiver) = match func.kind() {
            "identifier" => (self.text(func).to_string(), None),
            "attribute" => {
                let attr = func
                    .child_by_field_name("attribute")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let obj = func
                    .child_by_field_name("object")
                    .map(|n| self.text(n).to_string());
                (attr.to_string(), obj)
            }
            _ => (String::new(), None),
        };
        if name.is_empty() {
            return;
        }
        let receiver_kind = match receiver.as_deref() {
            None => "none",
            Some("self") | Some("cls") => "self",
            Some(_) => "value",
        };
        let arg_count = node.child_by_field_name("arguments").map(|a| {
            let mut c = a.walk();
            a.named_children(&mut c).count() as i64
        });
        self.out.refs.push(NewRef {
            name,
            ref_kind: "call".to_string(),
            receiver,
            start_line: self.line(node),
            start_byte: node.start_byte() as i64,
            arg_count,
            receiver_kind: receiver_kind.to_string(),
            ..Default::default()
        });
    }
}

/// `pkg.mod.Foo[int]` -> `Foo`; drops subscripts, unions, whitespace.
fn simple_type_name(raw: &str) -> String {
    let head = raw
        .trim()
        .trim_start_matches(['"', '\''])
        .split(['[', ' ', '|'])
        .next()
        .unwrap_or(raw)
        .trim_end_matches(['"', '\'']);
    head.rsplit('.').next().unwrap_or(head).to_string()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_python_symbols_imports_calls() {
        let src = r#"
import os
import numpy as np
from .cache import CacheDb
from ..util import helper, other as o

MAX_SIZE = 512

class Builder:
    def build(self):
        helper()
        self.step()

    def step(self):
        CacheDb.open()

def run():
    b = Builder()
    b.build()
"#;
        let p = parse(src);
        assert!(p.parse_ok);

        let sym = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(sym("Builder").unwrap().kind, "class");
        assert_eq!(sym("build").unwrap().kind, "method");
        assert_eq!(sym("run").unwrap().kind, "function");
        assert_eq!(sym("MAX_SIZE").unwrap().kind, "variable");

        let spec = |s: &str, n: &str| {
            p.imports
                .iter()
                .find(|i| i.raw_specifier == s && i.imported_name.as_deref() == Some(n))
        };
        assert!(spec(".cache", "CacheDb").unwrap().is_relative);
        assert!(spec("..util", "helper").unwrap().is_relative);
        assert_eq!(spec("..util", "other").unwrap().alias.as_deref(), Some("o"));
        assert!(p
            .imports
            .iter()
            .any(|i| i.raw_specifier == "numpy" && i.alias.as_deref() == Some("np")));

        let calls: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"helper"));
        assert!(calls.contains(&"build"));
        assert!(calls.contains(&"open"));
        assert!(calls.contains(&"Builder"));
    }
}
