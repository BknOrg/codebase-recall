//! JavaScript / TypeScript extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

use crate::analysis::scope::{self, ScopeStack};
use crate::analysis::{Language, ParsedFile};
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str, language: Language) -> ParsedFile {
    let ts_language: tree_sitter::Language = match language {
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    };

    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
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
            "function_declaration" | "generator_function_declaration"
            | "function_signature" => {
                self.enter_symbol(node, node, "function", false);
            }
            "class_declaration" | "abstract_class_declaration" | "class" => {
                self.enter_symbol(node, node, "class", false);
            }
            "interface_declaration" => self.enter_symbol(node, node, "interface", false),
            "type_alias_declaration" => self.leaf_symbol(node, node, "type", false),
            "enum_declaration" => self.enter_symbol(node, node, "enum", false),
            "method_definition" => self.enter_symbol(node, node, "method", false),
            "lexical_declaration" | "variable_declaration" => {
                self.collect_declarators(node);
            }
            "import_statement" => self.collect_import(node),
            "export_statement" => {
                // `export { a } from "..."` re-export: treat the source as an import.
                if let Some(src) = node.child_by_field_name("source") {
                    self.push_import(&unquote(self.text(src)), None, None, self.line(node));
                }
                self.walk_children(node);
            }
            "call_expression" => {
                self.collect_call(node);
                self.walk_children(node);
            }
            "new_expression" => {
                if let Some(c) = node.child_by_field_name("constructor") {
                    let (name, receiver) = self.callee_name(c);
                    if !name.is_empty() {
                        let rk = js_receiver_kind(receiver.as_deref());
                        let ac = self.arg_count(node);
                        self.push_ref(name, "call", receiver, rk, ac, node);
                    }
                }
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

    /// A declaration that can contain nested symbols.
    fn enter_symbol(&mut self, node: Node, name_host: Node, kind: &str, exported_hint: bool) {
        let idx = self.push_symbol(node, name_host, kind, exported_hint);
        if let Some(i) = idx {
            let (name, real_kind) = {
                let s = &self.out.symbols[i];
                (s.name.clone(), s.kind.clone())
            };
            let scope_kind = match real_kind.as_str() {
                "class" | "interface" => "class",
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
                "class" | "interface" => self.bind_class_fields(node),
                "function" | "method" => self.bind_params(node),
                _ => {}
            }

            self.walk_children(node);

            self.sc.pop();
            self.stack.pop();
        } else {
            self.walk_children(node);
        }
    }

    /// A declaration with no nested symbols worth tracking.
    fn leaf_symbol(&mut self, node: Node, name_host: Node, kind: &str, exported_hint: bool) {
        if let Some(i) = self.push_symbol(node, name_host, kind, exported_hint) {
            let name = self.out.symbols[i].name.clone();
            self.sc.bind(&name, "symbol", Some(i), None, None);
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
                "required_parameter" | "optional_parameter" => {
                    let n = p
                        .child_by_field_name("pattern")
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_default();
                    let t = p
                        .child_by_field_name("type")
                        .and_then(|ta| ta.named_child(0))
                        .map(|x| simple_type_name(self.text(x)));
                    (n, t)
                }
                "identifier" => (self.text(p).to_string(), None),
                _ => continue,
            };
            self.sc.bind(&name, "param", None, None, ty);
        }
    }

    fn bind_class_fields(&mut self, class: Node) {
        let Some(body) = class.child_by_field_name("body") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = body.walk();
            body.named_children(&mut c).collect()
        };
        for m in kids {
            if !matches!(m.kind(), "public_field_definition" | "property_signature") {
                continue;
            }
            let Some(n) = m.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(n).to_string();
            let ty = m
                .child_by_field_name("type")
                .and_then(|ta| ta.named_child(0))
                .map(|x| simple_type_name(self.text(x)));
            self.sc.bind(&name, "field", None, None, ty);
        }
    }

    fn param_count(&self, node: Node) -> Option<i64> {
        let params = node.child_by_field_name("parameters")?;
        let mut c = params.walk();
        let n = params
            .named_children(&mut c)
            .filter(|p| {
                matches!(
                    p.kind(),
                    "required_parameter" | "optional_parameter" | "identifier" | "rest_pattern"
                )
            })
            .count();
        Some(n as i64)
    }

    fn enclosing_type_name(&self) -> Option<String> {
        self.stack.iter().rev().find_map(|&i| {
            let s = &self.out.symbols[i];
            matches!(s.kind.as_str(), "class" | "interface").then(|| s.name.clone())
        })
    }

    fn push_symbol(
        &mut self,
        node: Node,
        name_host: Node,
        kind: &str,
        exported_hint: bool,
    ) -> Option<usize> {
        let name = name_host
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())?;

        let kind = if kind == "function" && self.parent_is_class() {
            "method"
        } else {
            kind
        };
        let exported = exported_hint || self.is_exported(node);

        let (param_count, type_name) = if matches!(kind, "function" | "method") {
            (
                self.param_count(node),
                self.enclosing_type_name().filter(|_| kind == "method"),
            )
        } else {
            (None, None)
        };

        self.out.symbols.push(NewSymbol {
            name,
            kind: kind.to_string(),
            parent_index: self.stack.last().copied(),
            is_exported: exported,
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

    fn parent_is_class(&self) -> bool {
        self.stack.last().is_some_and(|&i| {
            matches!(self.out.symbols[i].kind.as_str(), "class" | "interface")
        })
    }

    fn is_exported(&self, node: Node) -> bool {
        let mut cur = node.parent();
        for _ in 0..3 {
            match cur {
                Some(p) if p.kind() == "export_statement" => return true,
                Some(p) if matches!(p.kind(), "program" | "statement_block") => return false,
                Some(p) => cur = p.parent(),
                None => return false,
            }
        }
        false
    }

    fn collect_declarators(&mut self, decl: Node) {
        let exported = self.is_exported(decl);
        let children: Vec<Node> = {
            let mut c = decl.walk();
            decl.children(&mut c).collect()
        };
        for d in children {
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(name_node) = d.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name_node).to_string();
            if name.is_empty() {
                continue;
            }
            let value = d.child_by_field_name("value");
            let is_fn = matches!(
                value.map(|v| v.kind()),
                Some("arrow_function") | Some("function_expression") | Some("function")
            );
            let only_module_scope = self.stack.is_empty();
            let make_symbol = is_fn || only_module_scope;

            // A `const x: T = ...` inside a function body is a local, and its
            // declared type feeds field/receiver-type resolution.
            let decl_type = d
                .child_by_field_name("type")
                .and_then(|ta| ta.named_child(0))
                .map(|x| simple_type_name(self.text(x)));

            let mut pushed = None;
            if make_symbol {
                let kind = if is_fn { "function" } else { "variable" };
                self.out.symbols.push(NewSymbol {
                    name: name.clone(),
                    kind: kind.to_string(),
                    parent_index: self.stack.last().copied(),
                    is_exported: exported,
                    start_line: self.line(d),
                    end_line: d.end_position().row as i64 + 1,
                    start_byte: d.start_byte() as i64,
                    end_byte: d.end_byte() as i64,
                    signature: None,
                    param_count: None,
                    type_name: None,
                });
                pushed = Some(self.out.symbols.len() - 1);
                self.sc.bind(&name, "symbol", pushed, None, None);
            } else {
                self.sc.bind(&name, "local", None, None, decl_type);
            }

            // Always descend into the initializer so nested calls are captured.
            if let Some(v) = value {
                if let Some(i) = pushed.filter(|_| is_fn) {
                    self.stack.push(i);
                    self.walk_children(v);
                    self.stack.pop();
                } else {
                    self.walk(v);
                }
            }
        }
    }

    fn collect_import(&mut self, node: Node) {
        let line = self.line(node);
        let source = node
            .child_by_field_name("source")
            .map(|n| unquote(self.text(n)))
            .unwrap_or_default();
        if source.is_empty() {
            return;
        }

        let Some(clause) = first_child_of_kind(node, "import_clause") else {
            // side-effect import: `import "./x"`
            self.push_import(&source, None, None, line);
            return;
        };

        let mut any = false;
        let parts: Vec<Node> = {
            let mut c = clause.walk();
            clause.children(&mut c).collect()
        };
        for part in parts {
            match part.kind() {
                "identifier" => {
                    self.push_import(&source, Some("default"), Some(self.text(part)), line);
                    any = true;
                }
                "namespace_import" => {
                    let alias = part.named_child(0).map(|n| self.text(n));
                    self.push_import(&source, None, alias, line);
                    any = true;
                }
                "named_imports" => {
                    let specs: Vec<Node> = {
                        let mut c = part.walk();
                        part.named_children(&mut c).collect()
                    };
                    for spec in specs {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let name = spec.child_by_field_name("name").map(|n| self.text(n));
                        let alias = spec.child_by_field_name("alias").map(|n| self.text(n));
                        self.push_import(&source, name, alias.or(name), line);
                        any = true;
                    }
                }
                _ => {}
            }
        }
        if !any {
            self.push_import(&source, None, None, line);
        }
    }

    fn push_import(
        &mut self,
        specifier: &str,
        imported_name: Option<&str>,
        local: Option<&str>,
        line: i64,
    ) {
        let is_relative = specifier.starts_with('.') || specifier.starts_with('/');
        let imported_name = imported_name.map(str::to_string);
        let alias = match (local, &imported_name) {
            (Some(l), Some(n)) if l != n => Some(l.to_string()),
            (Some(l), None) => Some(l.to_string()),
            _ => None,
        };

        let import_index = self.out.imports.len();
        match (&imported_name, local) {
            // `import * as ns from "..."` -> namespace binding
            (None, Some(ns)) => {
                self.sc.bind(ns, "namespace", None, Some(import_index), None);
            }
            // `import { a as b }` / `import def` -> named binding under its local name
            (Some(n), l) => {
                self.sc
                    .bind(l.unwrap_or(n), "import", None, Some(import_index), None);
            }
            _ => {}
        }

        self.out.imports.push(NewImport {
            raw_specifier: specifier.to_string(),
            imported_name,
            alias,
            is_relative,
            start_line: line,
        });
    }

    fn collect_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        // CommonJS require("x")
        if func.kind() == "identifier" && self.text(func) == "require" {
            if let Some(args) = node.child_by_field_name("arguments") {
                if let Some(arg) = args.named_child(0) {
                    if arg.kind() == "string" {
                        let spec = unquote(self.text(arg));
                        self.push_import(&spec, None, None, self.line(node));
                        return;
                    }
                }
            }
        }
        let (name, receiver) = self.callee_name(func);
        if !name.is_empty() {
            let rk = js_receiver_kind(receiver.as_deref());
            let ac = self.arg_count(node);
            self.push_ref(name, "call", receiver, rk, ac, node);
        }
    }

    fn arg_count(&self, call: Node) -> Option<i64> {
        let args = call.child_by_field_name("arguments")?;
        let mut c = args.walk();
        Some(args.named_children(&mut c).count() as i64)
    }

    fn callee_name(&self, func: Node) -> (String, Option<String>) {
        match func.kind() {
            "identifier" => (self.text(func).to_string(), None),
            "member_expression" => {
                let prop = func
                    .child_by_field_name("property")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                let obj = func
                    .child_by_field_name("object")
                    .map(|n| self.text(n).to_string());
                (prop.to_string(), obj)
            }
            _ => (String::new(), None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_ref(
        &mut self,
        name: String,
        kind: &str,
        receiver: Option<String>,
        receiver_kind: &str,
        arg_count: Option<i64>,
        node: Node,
    ) {
        self.out.refs.push(NewRef {
            name,
            ref_kind: kind.to_string(),
            receiver,
            start_line: self.line(node),
            start_byte: node.start_byte() as i64,
            arg_count,
            receiver_kind: receiver_kind.to_string(),
            ..Default::default()
        });
    }
}

fn js_receiver_kind(receiver: Option<&str>) -> &'static str {
    match receiver {
        None => "none",
        Some("this") => "self",
        Some(_) => "value",
    }
}

/// `import("mod").Foo<T>` -> `Foo`; `a.b.C` -> `C`; drops generics/whitespace.
fn simple_type_name(raw: &str) -> String {
    let head = raw.trim().split(['<', ' ', '|', '&']).next().unwrap_or(raw).trim();
    head.rsplit('.').next().unwrap_or(head).to_string()
}

fn first_child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string()
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::analysis::Language;

    #[test]
    fn extracts_ts_symbols_imports_calls() {
        let src = r#"
import { CacheDb } from "./cache";
import helper from "../util/helper";
import * as fs from "node:fs";

export interface Options { depth: number; }

export class Builder {
    build(): void { helper(); this.step(); }
    step() { CacheDb.open(); }
}

export const run = () => {
    const b = new Builder();
    b.build();
};
"#;
        let p = parse(src, Language::TypeScript);
        assert!(p.parse_ok);

        let sym = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(sym("Builder").unwrap().kind, "class");
        assert!(sym("Builder").unwrap().is_exported);
        assert_eq!(sym("build").unwrap().kind, "method");
        assert_eq!(sym("Options").unwrap().kind, "interface");
        assert_eq!(sym("run").unwrap().kind, "function");

        let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);
        assert!(spec("./cache").unwrap().is_relative);
        assert_eq!(spec("./cache").unwrap().imported_name.as_deref(), Some("CacheDb"));
        assert_eq!(spec("../util/helper").unwrap().imported_name.as_deref(), Some("default"));
        assert!(!spec("node:fs").unwrap().is_relative);

        let calls: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"helper"));
        assert!(calls.contains(&"build"));
        assert!(calls.contains(&"open"));
        assert!(calls.contains(&"Builder")); // new Builder()

        // `helper()` inside `build` binds to the imported `helper`, not a local.
        let helper_ref = p.refs.iter().find(|r| r.name == "helper").unwrap();
        assert!(!helper_ref.local_only);
        assert!(helper_ref.resolved_local_symbol_index.is_none());
    }

    #[test]
    fn typed_class_fields_and_receiver_kinds() {
        let src = r#"
import { Pool } from "./pool";

class Server {
    private pool: Pool;
    name: string;

    handle(): void {
        this.pool.acquire();
        this.name;
    }
}
"#;
        let p = parse(src, Language::TypeScript);
        assert!(p.parse_ok);

        let pool_field = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "field" && b.name == "pool")
            .expect("pool field binding");
        assert_eq!(pool_field.type_expr.as_deref(), Some("Pool"));

        // `this.pool.acquire()` -> a value receiver call.
        let acq = p.refs.iter().find(|r| r.name == "acquire").unwrap();
        assert_eq!(acq.receiver.as_deref(), Some("this.pool"));
        assert_eq!(acq.receiver_kind, "value");

        // `handle` is a method whose owning type is `Server`.
        let handle = p.symbols.iter().find(|s| s.name == "handle").unwrap();
        assert_eq!(handle.kind, "method");
        assert_eq!(handle.type_name.as_deref(), Some("Server"));
    }
}
