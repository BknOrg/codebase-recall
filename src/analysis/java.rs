//! Java extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

use crate::analysis::ParsedFile;
use crate::analysis::scope::{self, ScopeStack};
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str) -> ParsedFile {
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
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
            "class_declaration" | "record_declaration" => self.enter_symbol(node, "class"),
            "interface_declaration" => self.enter_symbol(node, "interface"),
            "enum_declaration" => self.enter_symbol(node, "enum"),
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => self.enter_symbol(node, "method"),
            "field_declaration" | "constant_declaration" => {
                self.collect_field(node);
                self.walk_children(node);
            }
            "local_variable_declaration" => {
                self.collect_local(node);
                self.walk_children(node);
            }
            "import_declaration" => self.collect_import(node),
            "method_invocation" => {
                self.collect_call(node);
                self.walk_children(node);
            }
            "object_creation_expression" => {
                self.collect_new(node);
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

    fn enter_symbol(&mut self, node: Node, kind: &str) {
        let idx = self.push_symbol(node, kind);
        if let Some(i) = idx {
            let (name, real_kind) = {
                let s = &self.out.symbols[i];
                (s.name.clone(), s.kind.clone())
            };
            let scope_kind = match real_kind.as_str() {
                "method" => "method",
                _ => "class",
            };
            if real_kind != "method" {
                self.sc.bind(&name, "symbol", Some(i), None, None);
            }
            self.stack.push(i);
            self.sc.push(
                scope_kind,
                Some(i),
                node.start_byte() as i64,
                node.end_byte() as i64,
            );

            if real_kind == "method" {
                self.bind_params(node);
            }

            self.walk_children(node);

            self.sc.pop();
            self.stack.pop();
        } else {
            self.walk_children(node);
        }
    }

    fn bind_params(&mut self, node: Node) {
        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = params.walk();
            params.named_children(&mut c).collect()
        };
        for p in kids {
            if !matches!(p.kind(), "formal_parameter" | "spread_parameter") {
                continue;
            }
            let name = p
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .or_else(|| {
                    p.child_by_field_name("declarator")
                        .and_then(|d| d.child_by_field_name("name"))
                        .map(|n| self.text(n).to_string())
                })
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let ty = p
                .child_by_field_name("type")
                .map(|t| simple_type_name(self.text(t)));
            self.sc.bind(&name, "param", None, None, ty);
        }
    }

    fn param_count(&self, node: Node) -> Option<i64> {
        let params = node.child_by_field_name("parameters")?;
        let mut c = params.walk();
        Some(
            params
                .named_children(&mut c)
                .filter(|p| matches!(p.kind(), "formal_parameter" | "spread_parameter"))
                .count() as i64,
        )
    }

    fn enclosing_type_name(&self) -> Option<String> {
        self.stack.iter().rev().find_map(|&i| {
            let s = &self.out.symbols[i];
            matches!(s.kind.as_str(), "class" | "interface" | "enum").then(|| s.name.clone())
        })
    }

    fn is_public(&self, node: Node) -> bool {
        node.child_by_field_name("modifiers")
            .or_else(|| first_child_of_kind(node, "modifiers"))
            .is_some_and(|m| {
                let mut c = m.walk();
                m.children(&mut c).any(|x| x.kind() == "public")
            })
    }

    fn push_symbol(&mut self, node: Node, kind: &str) -> Option<usize> {
        // Constructors: name node is the type name; treat the method name as `<init>`
        // would be noisy, so use the type name (matches "call FooCtor" heuristics).
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())?;

        let (param_count, type_name) = if kind == "method" {
            (self.param_count(node), self.enclosing_type_name())
        } else {
            (None, None)
        };

        self.out.symbols.push(NewSymbol {
            name,
            kind: kind.to_string(),
            parent_index: self.stack.last().copied(),
            is_exported: self.is_public(node) || kind != "method" && self.stack.is_empty(),
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

    /// `Type a, b;` field(s) -> field bindings carrying the declared type.
    fn collect_field(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let tname = simple_type_name(self.text(ty));
        let decls: Vec<Node> = {
            let mut c = node.walk();
            node.children_by_field_name("declarator", &mut c).collect()
        };
        for d in decls {
            if let Some(n) = d.child_by_field_name("name") {
                let name = self.text(n).to_string();
                if !name.is_empty() {
                    self.sc
                        .bind(&name, "field", None, None, Some(tname.clone()));
                }
            }
        }
    }

    /// `Type v = ...;` inside a block -> local binding with its declared type.
    fn collect_local(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let tname = simple_type_name(self.text(ty));
        let decls: Vec<Node> = {
            let mut c = node.walk();
            node.children_by_field_name("declarator", &mut c).collect()
        };
        for d in decls {
            if let Some(n) = d.child_by_field_name("name") {
                let name = self.text(n).to_string();
                if !name.is_empty() {
                    self.sc
                        .bind(&name, "local", None, None, Some(tname.clone()));
                }
            }
        }
    }

    fn collect_import(&mut self, node: Node) {
        let line = self.line(node);
        let is_wildcard = first_child_of_kind(node, "asterisk").is_some();
        let path_node = first_child_of_kind(node, "scoped_identifier")
            .or_else(|| first_child_of_kind(node, "identifier"));
        let Some(path_node) = path_node else {
            return;
        };
        let path = self.text(path_node).to_string();
        if path.is_empty() {
            return;
        }

        if is_wildcard {
            // `import a.b.*;` — no single target; record for external rollup only.
            self.out.imports.push(NewImport {
                raw_specifier: path,
                imported_name: None,
                alias: None,
                is_relative: false,
                start_line: line,
            });
            return;
        }

        let last = path.rsplit('.').next().unwrap_or(&path).to_string();
        let import_index = self.out.imports.len();
        self.sc
            .bind(&last, "import", None, Some(import_index), None);
        self.out.imports.push(NewImport {
            raw_specifier: path,
            imported_name: Some(last),
            alias: None,
            is_relative: false,
            start_line: line,
        });
    }

    fn collect_call(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let receiver = node
            .child_by_field_name("object")
            .map(|o| self.text(o).to_string());
        let receiver_kind = match receiver.as_deref() {
            None => "none",
            Some("this") | Some("super") => "self",
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

    fn collect_new(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let name = simple_type_name(self.text(ty));
        if name.is_empty() {
            return;
        }
        let arg_count = node.child_by_field_name("arguments").map(|a| {
            let mut c = a.walk();
            a.named_children(&mut c).count() as i64
        });
        self.out.refs.push(NewRef {
            name,
            ref_kind: "call".to_string(),
            receiver: None,
            start_line: self.line(node),
            start_byte: node.start_byte() as i64,
            arg_count,
            receiver_kind: "none".to_string(),
            ..Default::default()
        });
    }
}

/// `java.util.Map<String, T>[]` -> `Map`; drops generics, arrays, package.
fn simple_type_name(raw: &str) -> String {
    let head = raw
        .trim()
        .split(['<', ' ', '[', '&'])
        .next()
        .unwrap_or(raw)
        .trim();
    head.rsplit('.').next().unwrap_or(head).to_string()
}

fn first_child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_java_symbols_imports_calls() {
        let src = r#"
package com.example.app;

import com.example.util.Greeter;
import com.example.data.*;
import static com.example.util.Helpers.log;

public class App {
    private Greeter greeter;

    public static void main(String[] args) {
        Greeter g = new Greeter("hi");
        g.greet();
        helper();
        log("done");
    }

    void helper() {}
}
"#;
        let p = parse(src);
        assert!(p.parse_ok);

        let sym = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(sym("App").unwrap().kind, "class");
        assert!(sym("App").unwrap().is_exported);
        assert_eq!(sym("main").unwrap().kind, "method");
        assert_eq!(sym("main").unwrap().type_name.as_deref(), Some("App"));
        assert_eq!(sym("helper").unwrap().kind, "method");
        assert!(!sym("helper").unwrap().is_exported);

        let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);
        assert_eq!(
            spec("com.example.util.Greeter")
                .unwrap()
                .imported_name
                .as_deref(),
            Some("Greeter")
        );
        assert!(!spec("com.example.util.Greeter").unwrap().is_relative);
        assert!(spec("com.example.data").unwrap().imported_name.is_none());
        assert_eq!(
            spec("com.example.util.Helpers.log")
                .unwrap()
                .imported_name
                .as_deref(),
            Some("log")
        );

        let calls: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"greet"));
        assert!(calls.contains(&"helper"));
        assert!(calls.contains(&"log"));
        assert!(calls.contains(&"Greeter")); // new Greeter(...)

        // `helper()` inside `main` resolves in-file to the sibling method.
        let helper_ref = p.refs.iter().find(|r| r.name == "helper").unwrap();
        assert_eq!(
            helper_ref.receiver, None,
            "bare call should have no receiver"
        );

        // `g` is a typed local; `greet` call carries a value receiver.
        let g_field = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "local" && b.name == "g");
        assert_eq!(g_field.unwrap().type_expr.as_deref(), Some("Greeter"));
        let greet = p.refs.iter().find(|r| r.name == "greet").unwrap();
        assert_eq!(greet.receiver.as_deref(), Some("g"));
        assert_eq!(greet.receiver_kind, "value");
    }

    #[test]
    fn field_declarations_become_typed_bindings() {
        let src = r#"
class Server {
    private Pool pool;
    String name;
}
"#;
        let p = parse(src);
        let field = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "field" && b.name == "pool")
            .expect("pool field binding");
        assert_eq!(field.type_expr.as_deref(), Some("Pool"));
    }
}
