//! Kotlin extraction via a tree-sitter AST walk.
//!
//! Covers ordinary Kotlin: top-level and member functions, classes / objects /
//! interfaces, properties, `import` (incl. `as` aliases and `.*`), and calls.
//! Jetpack Compose needs no special handling — a `@Composable fun` is an ordinary
//! function symbol and a composable call is an ordinary `calls` edge. Kotlin
//! Multiplatform (`expect`/`actual`, source sets) is out of scope.

use tree_sitter::{Node, Parser};

use crate::analysis::scope::{self, ScopeStack};
use crate::analysis::ParsedFile;
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str) -> ParsedFile {
    let language: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
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
            "class_declaration" | "object_declaration" => {
                let kind = if self.text(node).trim_start().starts_with("interface")
                    || first_child_of_kind(node, "interface").is_some()
                {
                    "interface"
                } else {
                    "class"
                };
                self.enter_symbol(node, kind);
            }
            "function_declaration" => {
                let kind = if self.parent_is_type() { "method" } else { "function" };
                self.enter_symbol(node, kind);
            }
            "property_declaration" => {
                self.collect_property(node);
                self.walk_children(node);
            }
            "import" => self.collect_import(node),
            "call_expression" => {
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

    fn parent_is_type(&self) -> bool {
        self.stack
            .last()
            .is_some_and(|&i| matches!(self.out.symbols[i].kind.as_str(), "class" | "interface"))
    }

    fn enter_symbol(&mut self, node: Node, kind: &str) {
        let idx = self.push_symbol(node, kind);
        if let Some(i) = idx {
            let (name, real_kind) = {
                let s = &self.out.symbols[i];
                (s.name.clone(), s.kind.clone())
            };
            let scope_kind = if real_kind == "method" { "method" } else { "class" };
            if real_kind != "method" {
                self.sc.bind(&name, "symbol", Some(i), None, None);
            }
            self.stack.push(i);
            self.sc
                .push(scope_kind, Some(i), node.start_byte() as i64, node.end_byte() as i64);

            match real_kind.as_str() {
                "method" | "function" => self.bind_params(node),
                "class" | "interface" => self.bind_primary_ctor(node),
                _ => {}
            }

            self.walk_children(node);

            self.sc.pop();
            self.stack.pop();
        } else {
            self.walk_children(node);
        }
    }

    fn bind_params(&mut self, node: Node) {
        let Some(params) = first_child_of_kind(node, "function_value_parameters") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = params.walk();
            params.named_children(&mut c).collect()
        };
        for p in kids {
            if p.kind() != "parameter" {
                continue;
            }
            let name = first_child_of_kind(p, "identifier")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let ty = p.named_children(&mut p.walk()).find_map(|c| {
                (c.kind() != "identifier").then(|| simple_type_name(self.text(c)))
            });
            self.sc.bind(&name, "param", None, None, ty);
        }
    }

    /// `class Foo(val bar: Bar)` — primary-constructor `val`/`var` params are fields.
    fn bind_primary_ctor(&mut self, node: Node) {
        let Some(ctor) = first_child_of_kind(node, "primary_constructor") else {
            return;
        };
        let Some(params) = first_child_of_kind(ctor, "class_parameters") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = params.walk();
            params.named_children(&mut c).collect()
        };
        for p in kids {
            if p.kind() != "class_parameter" {
                continue;
            }
            let name = first_child_of_kind(p, "identifier")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            // A plain constructor param (no val/var) is still worth a param binding.
            let is_field = {
                let mut c = p.walk();
                p.children(&mut c).any(|x| matches!(x.kind(), "val" | "var"))
            };
            let ty = p.named_children(&mut p.walk()).find_map(|c| {
                (c.kind() != "identifier").then(|| simple_type_name(self.text(c)))
            });
            self.sc
                .bind(&name, if is_field { "field" } else { "param" }, None, None, ty);
        }
    }

    fn enclosing_type_name(&self) -> Option<String> {
        self.stack.iter().rev().find_map(|&i| {
            let s = &self.out.symbols[i];
            matches!(s.kind.as_str(), "class" | "interface").then(|| s.name.clone())
        })
    }

    fn is_exported(&self, node: Node) -> bool {
        // Kotlin defaults to public; only an explicit narrower modifier hides it.
        match first_child_of_kind(node, "modifiers") {
            Some(m) => {
                let t = self.text(m);
                !t.contains("private") && !t.contains("protected") && !t.contains("internal")
            }
            None => true,
        }
    }

    fn param_count(&self, node: Node) -> Option<i64> {
        let params = first_child_of_kind(node, "function_value_parameters")?;
        let mut c = params.walk();
        Some(
            params
                .named_children(&mut c)
                .filter(|p| p.kind() == "parameter")
                .count() as i64,
        )
    }

    fn push_symbol(&mut self, node: Node, kind: &str) -> Option<usize> {
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
            is_exported: self.is_exported(node),
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

    fn collect_property(&mut self, node: Node) {
        let Some(vd) = first_child_of_kind(node, "variable_declaration") else {
            return;
        };
        let Some(name_node) = first_child_of_kind(vd, "identifier") else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }

        // Declared type: annotation on the variable, or on the property itself.
        let declared = vd
            .named_children(&mut vd.walk())
            .find(|c| c.kind() != "identifier")
            .or_else(|| first_child_of_kind(node, "user_type"))
            .map(|t| simple_type_name(self.text(t)));
        // Otherwise infer from `= Ctor(...)`.
        let ty = declared.or_else(|| self.ctor_type_of_initializer(node));

        let at_module = self.stack.is_empty();
        let in_type = self.parent_is_type();

        if at_module {
            self.out.symbols.push(NewSymbol {
                name: name.clone(),
                kind: "variable".to_string(),
                parent_index: None,
                is_exported: self.is_exported(node),
                start_line: self.line(node),
                end_line: node.end_position().row as i64 + 1,
                start_byte: node.start_byte() as i64,
                end_byte: node.end_byte() as i64,
                signature: None,
                param_count: None,
                type_name: None,
            });
            let idx = self.out.symbols.len() - 1;
            self.sc.bind(&name, "symbol", Some(idx), None, None);
        } else if in_type {
            self.sc.bind(&name, "field", None, None, ty);
        } else {
            self.sc.bind(&name, "local", None, None, ty);
        }
    }

    /// `val x = Foo(...)` -> `Some("Foo")` when the callee looks like a type.
    fn ctor_type_of_initializer(&self, prop: Node) -> Option<String> {
        let call = first_child_of_kind(prop, "call_expression")?;
        let callee = call.named_child(0)?;
        if callee.kind() != "identifier" {
            return None;
        }
        let t = self.text(callee);
        t.chars().next().is_some_and(|c| c.is_uppercase()).then(|| t.to_string())
    }

    fn collect_import(&mut self, node: Node) {
        let line = self.line(node);
        let Some(qi) = first_child_of_kind(node, "qualified_identifier") else {
            return;
        };
        let path = self.text(qi).to_string();
        if path.is_empty() {
            return;
        }
        let raw = self.text(node);
        let is_wildcard = raw.trim_end().ends_with('*');
        let alias = node
            .children(&mut node.walk())
            .skip_while(|c| c.kind() != "as")
            .nth(1)
            .filter(|c| c.kind() == "identifier")
            .map(|c| self.text(c).to_string());

        if is_wildcard {
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
        let local = alias.clone().unwrap_or_else(|| last.clone());
        let import_index = self.out.imports.len();
        self.sc
            .bind(&local, "import", None, Some(import_index), None);
        self.out.imports.push(NewImport {
            raw_specifier: path,
            imported_name: Some(last),
            alias,
            is_relative: false,
            start_line: line,
        });
    }

    fn collect_call(&mut self, node: Node) {
        let Some(callee) = node.named_child(0) else {
            return;
        };
        let (name, receiver) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), None),
            "navigation_expression" => {
                let ids: Vec<Node> = {
                    let mut c = callee.walk();
                    callee
                        .children(&mut c)
                        .filter(|x| x.kind() != "." && x.is_named())
                        .collect()
                };
                let method = ids.last().map(|n| self.text(*n).to_string()).unwrap_or_default();
                let recv_node = callee.child(0);
                let receiver = recv_node.map(|n| self.text(n).to_string());
                (method, receiver)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        let receiver_kind = match receiver.as_deref() {
            None => "none",
            Some("this") | Some("super") => "self",
            Some(_) => "value",
        };
        let arg_count = first_child_of_kind(node, "value_arguments").map(|a| {
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

/// `com.example.Foo<Bar>?` -> `Foo`; drops nullability, generics, package.
fn simple_type_name(raw: &str) -> String {
    let head = raw
        .trim()
        .trim_end_matches('?')
        .split(['<', ' ', '?', '&'])
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
    fn extracts_kotlin_symbols_imports_calls() {
        let src = r#"
package com.example.app

import com.example.util.Greeter
import com.example.util.Helpers.log as logIt
import com.example.data.*

class Screen(val greeter: Greeter) {
    fun render() {
        val g = Greeter("hi")
        g.greet()
        helper()
        logIt("x")
    }
    fun helper() {}
}

@Composable
fun Content() {
    val s = Screen(Greeter())
    s.render()
}

val TOP = 42
"#;
        let p = parse(src);
        assert!(p.parse_ok);

        let sym = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(sym("Screen").unwrap().kind, "class");
        assert_eq!(sym("render").unwrap().kind, "method");
        assert_eq!(sym("render").unwrap().type_name.as_deref(), Some("Screen"));
        // A top-level @Composable function is an ordinary function symbol.
        assert_eq!(sym("Content").unwrap().kind, "function");
        assert_eq!(sym("TOP").unwrap().kind, "variable");

        let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);
        assert_eq!(
            spec("com.example.util.Greeter").unwrap().imported_name.as_deref(),
            Some("Greeter")
        );
        assert_eq!(
            spec("com.example.util.Helpers.log").unwrap().alias.as_deref(),
            Some("logIt")
        );
        assert!(spec("com.example.data").unwrap().imported_name.is_none());

        let calls: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"greet"));
        assert!(calls.contains(&"helper"));
        assert!(calls.contains(&"render"));
        assert!(calls.contains(&"Greeter")); // Greeter("hi") constructor call

        // `val greeter: Greeter` primary-ctor property -> typed field binding.
        let greeter_field = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "field" && b.name == "greeter")
            .expect("greeter field binding");
        assert_eq!(greeter_field.type_expr.as_deref(), Some("Greeter"));

        // `val g = Greeter("hi")` -> local binding with inferred ctor type.
        let g_local = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "local" && b.name == "g")
            .expect("g local binding");
        assert_eq!(g_local.type_expr.as_deref(), Some("Greeter"));

        let greet = p.refs.iter().find(|r| r.name == "greet").unwrap();
        assert_eq!(greet.receiver.as_deref(), Some("g"));
        assert_eq!(greet.receiver_kind, "value");
    }
}
