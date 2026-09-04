//! JavaScript / TypeScript extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

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
    };
    walker.walk(tree.root_node());
    walker.out
}

struct Walker<'a> {
    src: &'a [u8],
    out: ParsedFile,
    stack: Vec<usize>,
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
                        self.push_ref(name, "call", receiver, node);
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
            self.stack.push(i);
        }
        self.walk_children(node);
        if idx.is_some() {
            self.stack.pop();
        }
    }

    /// A declaration with no nested symbols worth tracking.
    fn leaf_symbol(&mut self, node: Node, name_host: Node, kind: &str, exported_hint: bool) {
        self.push_symbol(node, name_host, kind, exported_hint);
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

            let mut pushed = None;
            if make_symbol {
                let kind = if is_fn { "function" } else { "variable" };
                self.out.symbols.push(NewSymbol {
                    name,
                    kind: kind.to_string(),
                    parent_index: self.stack.last().copied(),
                    is_exported: exported,
                    start_line: self.line(d),
                    end_line: d.end_position().row as i64 + 1,
                    start_byte: d.start_byte() as i64,
                    end_byte: d.end_byte() as i64,
                    signature: None,
                });
                pushed = Some(self.out.symbols.len() - 1);
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
            self.push_ref(name, "call", receiver, node);
        }
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

    fn push_ref(&mut self, name: String, kind: &str, receiver: Option<String>, node: Node) {
        self.out.refs.push(NewRef {
            name,
            ref_kind: kind.to_string(),
            receiver,
            start_line: self.line(node),
            start_byte: node.start_byte() as i64,
        });
    }
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
    }
}
