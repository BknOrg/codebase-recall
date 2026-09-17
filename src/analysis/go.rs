//! Go extraction via a tree-sitter AST walk.

use tree_sitter::{Node, Parser};

use crate::analysis::ParsedFile;
use crate::analysis::scope::{self, ScopeStack};
use crate::cache::models::{NewImport, NewRef, NewSymbol};

pub fn parse(source: &str) -> ParsedFile {
    let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
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
        pkg_name: String::new(),
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
    pkg_name: String,
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
            "package_clause" => {
                if let Some(id) = node.child_by_field_name("package") {
                    self.pkg_name = self.text(id).to_string();
                } else if let Some(id) = first_child_of_kind(node, "package_identifier") {
                    self.pkg_name = self.text(id).to_string();
                }
            }
            "import_declaration" => {
                self.collect_imports(node);
            }
            "function_declaration" => {
                self.enter_function(node);
            }
            "method_declaration" => {
                self.enter_method(node);
            }
            "type_declaration" => {
                self.collect_type_declaration(node);
            }
            "var_declaration" | "const_declaration" => {
                self.collect_vars_and_consts(node);
                self.walk_children(node);
            }
            "short_var_declaration" => {
                self.collect_short_var(node);
                self.walk_children(node);
            }
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

    fn collect_imports(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import_spec" {
                self.record_import_spec(child);
            } else if child.kind() == "import_spec_list" {
                let mut inner_cursor = child.walk();
                for spec in child.children(&mut inner_cursor) {
                    if spec.kind() == "import_spec" {
                        self.record_import_spec(spec);
                    }
                }
            }
        }
    }

    fn record_import_spec(&mut self, spec: Node) {
        let Some(path_node) = spec.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path_node).trim_matches('"').trim_matches('`');
        if raw.is_empty() {
            return;
        }

        let alias = spec
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string());

        let default_name = raw.rsplit('/').next().unwrap_or(raw).to_string();
        let bound_name = match &alias {
            Some(a) if a == "_" || a == "." => None,
            Some(a) => Some(a.clone()),
            None => Some(default_name.clone()),
        };

        let import_index = self.out.imports.len();
        if let Some(name) = bound_name {
            self.sc
                .bind(&name, "import", None, Some(import_index), None);
        }

        self.out.imports.push(NewImport {
            raw_specifier: raw.to_string(),
            imported_name: Some(default_name),
            alias,
            is_relative: raw.starts_with('.'),
            start_line: self.line(spec),
        });
    }

    fn enter_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.walk_children(node);
            return;
        };
        let name = self.text(name_node).to_string();
        let sig = self.signature_of(node);

        let param_count = node.child_by_field_name("parameters").map(|p| {
            let mut c = p.walk();
            p.children(&mut c)
                .filter(|ch| ch.kind() == "parameter_declaration")
                .count() as i64
        });

        let idx = self.push_symbol_raw(node, &name, "function", sig, param_count, None);
        if let Some(i) = idx {
            self.sc.bind(&name, "symbol", Some(i), None, None);
            self.stack.push(i);
            self.sc.push(
                "function",
                Some(i),
                node.start_byte() as i64,
                node.end_byte() as i64,
            );

            if let Some(params) = node.child_by_field_name("parameters") {
                self.bind_parameters(params);
            }

            if let Some(body) = node.child_by_field_name("body") {
                self.walk_children(body);
            }

            self.sc.pop();
            self.stack.pop();
        }
    }

    fn enter_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.walk_children(node);
            return;
        };
        let name = self.text(name_node).to_string();
        let sig = self.signature_of(node);

        let parent_type = self.extract_receiver_type(node);
        let param_count = node.child_by_field_name("parameters").map(|p| {
            let mut c = p.walk();
            p.children(&mut c)
                .filter(|ch| ch.kind() == "parameter_declaration")
                .count() as i64
        });

        let idx = self.push_symbol_raw(node, &name, "method", sig, param_count, parent_type);
        if let Some(i) = idx {
            self.stack.push(i);
            self.sc.push(
                "method",
                Some(i),
                node.start_byte() as i64,
                node.end_byte() as i64,
            );

            if let Some(recv) = node.child_by_field_name("receiver") {
                self.bind_parameters(recv);
            }
            if let Some(params) = node.child_by_field_name("parameters") {
                self.bind_parameters(params);
            }

            if let Some(body) = node.child_by_field_name("body") {
                self.walk_children(body);
            }

            self.sc.pop();
            self.stack.pop();
        }
    }

    fn extract_receiver_type(&self, node: Node) -> Option<String> {
        let recv = node.child_by_field_name("receiver")?;
        let mut cursor = recv.walk();
        for child in recv.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                if let Some(t) = child.child_by_field_name("type") {
                    let txt = self.text(t).trim_start_matches('*').trim();
                    return Some(txt.to_string());
                }
            }
        }
        None
    }

    fn collect_type_declaration(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_spec" {
                self.enter_type_spec(child);
            } else if child.kind() == "type_spec_list" {
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if spec.kind() == "type_spec" {
                        self.enter_type_spec(spec);
                    }
                }
            }
        }
    }

    fn enter_type_spec(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_string();
        let type_node = node.child_by_field_name("type");

        let kind = match type_node.map(|t| t.kind()) {
            Some("struct_type") => "struct",
            Some("interface_type") => "interface",
            _ => "type",
        };

        let sig = self.signature_of(node);
        let idx = self.push_symbol_raw(node, &name, kind, sig, None, None);
        if let Some(i) = idx {
            self.sc.bind(&name, "symbol", Some(i), None, None);
            self.stack.push(i);
            self.sc.push(
                kind,
                Some(i),
                node.start_byte() as i64,
                node.end_byte() as i64,
            );

            if let Some(tn) = type_node {
                if tn.kind() == "struct_type" {
                    if let Some(fields) = tn.child_by_field_name("fields") {
                        self.collect_struct_fields(fields);
                    }
                } else if tn.kind() == "interface_type" {
                    self.collect_interface_methods(tn);
                }
            }

            self.sc.pop();
            self.stack.pop();
        }
    }

    fn collect_struct_fields(&mut self, fields_node: Node) {
        let mut cursor = fields_node.walk();
        for child in fields_node.children(&mut cursor) {
            if child.kind() == "field_declaration" {
                let type_str = child
                    .child_by_field_name("type")
                    .map(|t| self.text(t).to_string());
                let mut fcursor = child.walk();
                for fchild in child.children(&mut fcursor) {
                    if fchild.kind() == "field_identifier" {
                        let fname = self.text(fchild);
                        self.sc.bind(fname, "field", None, None, type_str.clone());
                    }
                }
            }
        }
    }

    fn collect_interface_methods(&mut self, iface_node: Node) {
        let mut cursor = iface_node.walk();
        for child in iface_node.children(&mut cursor) {
            if child.kind() == "method_elem" || child.kind() == "method_spec" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let mname = self.text(name_node);
                    let sig = self.signature_of(child);
                    let param_count = child.child_by_field_name("parameters").map(|p| {
                        let mut c = p.walk();
                        p.children(&mut c)
                            .filter(|ch| ch.kind() == "parameter_declaration")
                            .count() as i64
                    });
                    self.push_symbol_raw(child, mname, "method", sig, param_count, None);
                }
            }
        }
    }

    fn bind_parameters(&mut self, params_node: Node) {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                let type_str = child
                    .child_by_field_name("type")
                    .map(|t| self.text(t).to_string());
                let mut pcursor = child.walk();
                for pchild in child.children(&mut pcursor) {
                    if pchild.kind() == "identifier" {
                        let pname = self.text(pchild);
                        self.sc.bind(pname, "param", None, None, type_str.clone());
                    }
                }
            }
        }
    }

    fn collect_vars_and_consts(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "var_spec" || child.kind() == "const_spec" {
                let type_str = child
                    .child_by_field_name("type")
                    .map(|t| self.text(t).to_string());
                let mut vcursor = child.walk();
                for vchild in child.children(&mut vcursor) {
                    if vchild.kind() == "identifier" {
                        let vname = self.text(vchild);
                        self.sc.bind(vname, "local", None, None, type_str.clone());
                    }
                }
            }
        }
    }

    fn collect_short_var(&mut self, node: Node) {
        if let Some(left) = node.child_by_field_name("left") {
            let mut cursor = left.walk();
            for child in left.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = self.text(child);
                    self.sc.bind(name, "local", None, None, None);
                }
            }
        }
    }

    fn collect_call(&mut self, node: Node) {
        let Some(func_node) = node.child_by_field_name("function") else {
            return;
        };

        let (name_node, receiver) = match func_node.kind() {
            "identifier" => (Some(func_node), None),
            "selector_expression" => {
                let field = func_node.child_by_field_name("field");
                let operand = func_node
                    .child_by_field_name("operand")
                    .map(|n| self.text(n).to_string());
                (field, operand)
            }
            _ => (None, None),
        };

        let Some(name_node) = name_node else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }

        let receiver_kind = match receiver.as_deref() {
            None => "none",
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
            name_start_byte: Some(name_node.start_byte() as i64),
            arg_count,
            receiver_kind: receiver_kind.to_string(),
            ..Default::default()
        });
    }

    fn push_symbol_raw(
        &mut self,
        node: Node,
        name: &str,
        kind: &str,
        signature: Option<String>,
        param_count: Option<i64>,
        type_name: Option<String>,
    ) -> Option<usize> {
        let start_line = self.line(node);
        let end_line = node.end_position().row as i64 + 1;
        let parent_index = self.stack.last().copied();
        let idx = self.out.symbols.len();

        let is_exported = name.chars().next().is_some_and(|c| c.is_uppercase());

        self.out.symbols.push(NewSymbol {
            name: name.to_string(),
            kind: kind.to_string(),
            parent_index,
            is_exported,
            start_line,
            end_line,
            start_byte: node.start_byte() as i64,
            end_byte: node.end_byte() as i64,
            signature,
            param_count,
            type_name,
        });
        Some(idx)
    }

    fn signature_of(&self, node: Node) -> Option<String> {
        let start = node.start_byte();
        let end = if let Some(body) = node.child_by_field_name("body") {
            body.start_byte()
        } else {
            node.end_byte()
        };
        if end > start && end <= self.src.len() {
            let s = std::str::from_utf8(&self.src[start..end]).ok()?;
            Some(s.trim().to_string())
        } else {
            None
        }
    }
}

fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_go_symbols_imports_calls() {
        let src = r#"
package main

import (
    "fmt"
    "net/http"
    custom "github.com/example/pkg"
)

// Server handles HTTP traffic
type Server struct {
    port int
}

type Router interface {
    Route(path string)
}

func (s *Server) Start() error {
    fmt.Println("starting server")
    custom.Init()
    s.handle()
    return nil
}

func (s *Server) handle() {
}

func main() {
    srv := &Server{port: 8080}
    srv.Start()
}
"#;
        let res = parse(src);
        assert!(res.parse_ok);

        // Check symbols
        let sym_names: Vec<&str> = res.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(sym_names.contains(&"Server"));
        assert!(sym_names.contains(&"Router"));
        assert!(sym_names.contains(&"Start"));
        assert!(sym_names.contains(&"handle"));
        assert!(sym_names.contains(&"main"));

        // Check imports
        let imports: Vec<&str> = res
            .imports
            .iter()
            .map(|i| i.raw_specifier.as_str())
            .collect();
        assert!(imports.contains(&"fmt"));
        assert!(imports.contains(&"net/http"));
        assert!(imports.contains(&"github.com/example/pkg"));

        // Check calls
        let call_names: Vec<&str> = res.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(call_names.contains(&"Println"));
        assert!(call_names.contains(&"Init"));
        assert!(call_names.contains(&"Start"));
        assert!(call_names.contains(&"handle"));
    }
}
