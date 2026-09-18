use tree_sitter::Node;

use crate::cache::models::{NewRef, NewStringLiteral};

use super::Walker;
use super::helpers::{classify_rust_receiver, clean_rust_string_literal};

impl<'a> Walker<'a> {
    pub fn collect_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let (name_node, receiver) = self.callee_parts(func);
        let name = name_node.map(|n| self.text(n).to_string()).unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let receiver_kind = classify_rust_receiver(func.kind(), receiver.as_deref());
        let arg_count = node
            .child_by_field_name("arguments")
            .map(|a| {
                let mut c = a.walk();
                a.named_children(&mut c).count() as i64
            });
        if let Some(args_node) = node.child_by_field_name("arguments") {
            let mut c = args_node.walk();
            for arg in args_node.named_children(&mut c) {
                self.check_and_record_string_literal(arg, Some(&name));
            }
        }
        self.out.refs.push(NewRef {
            name,
            ref_kind: "call".to_string(),
            receiver,
            start_line: node.start_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
            name_start_byte: name_node.map(|n| n.start_byte() as i64),
            arg_count,
            receiver_kind: receiver_kind.to_string(),
            ..Default::default()
        });
    }

    /// The callee's name node and its receiver text. The node — not just the
    /// text — because `--precise` asks the language server about that exact
    /// offset, and `a.b()` must point at `b`, not at `a`.
    pub fn callee_parts(&self, func: Node<'a>) -> (Option<Node<'a>>, Option<String>) {
        match func.kind() {
            "identifier" => (Some(func), None),
            "field_expression" => {
                let receiver = func
                    .child_by_field_name("value")
                    .map(|n| self.text(n).to_string());
                (func.child_by_field_name("field"), receiver)
            }
            "scoped_identifier" => {
                let path = func
                    .child_by_field_name("path")
                    .map(|n| self.text(n).to_string());
                (func.child_by_field_name("name"), path)
            }
            "generic_function" => func
                .child_by_field_name("function")
                .map(|inner| self.callee_parts(inner))
                .unwrap_or((None, None)),
            _ => (None, None),
        }
    }

    pub fn collect_macro(&mut self, node: Node) {
        let Some(m) = node.child_by_field_name("macro") else {
            return;
        };
        let name = self.text(m);
        if name.is_empty() {
            return;
        }
        let macro_name = format!("{name}!");
        // `token_tree` is a child kind in tree-sitter-rust, not a field name.
        let token_tree = {
            let mut c = node.walk();
            node.named_children(&mut c).find(|n| n.kind() == "token_tree")
        };
        if let Some(token_tree) = token_tree {
            let mut c = token_tree.walk();
            for child in token_tree.named_children(&mut c) {
                self.check_and_record_string_literal(child, Some(&macro_name));
            }
        }
        self.out.refs.push(NewRef {
            name: macro_name,
            ref_kind: "call".to_string(),
            receiver: None,
            start_line: node.start_position().row as i64 + 1,
            start_byte: node.start_byte() as i64,
            name_start_byte: Some(m.start_byte() as i64),
            receiver_kind: "none".to_string(),
            ..Default::default()
        });
    }

    pub fn check_and_record_string_literal(&mut self, node: Node, callee: Option<&str>) {
        if node.kind() == "string_literal" || node.kind() == "raw_string_literal" {
            let text = self.text(node);
            let unquoted = clean_rust_string_literal(text);
            if !unquoted.is_empty() {
                self.out.string_literals.push(NewStringLiteral {
                    value: unquoted,
                    callee: callee.map(String::from),
                    line: Some(node.start_position().row as i64 + 1),
                });
            }
        }
    }
}
