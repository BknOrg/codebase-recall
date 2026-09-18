use tree_sitter::Node;

use crate::cache::models::NewSymbol;

use super::Walker;
use super::helpers::simple_type_name;

impl<'a> Walker<'a> {
    pub fn push_symbol(&mut self, node: Node) -> Option<usize> {
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
        let (param_count, type_name) = if kind == "method" || kind == "function" {
            (self.param_count(node), self.enclosing_type_name().filter(|_| kind == "method"))
        } else {
            (None, None)
        };
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
            param_count,
            type_name,
        });
        Some(self.out.symbols.len() - 1)
    }

    pub fn bind_params(&mut self, func: Node) {
        let Some(params) = func.child_by_field_name("parameters") else {
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
            let Some(pat) = p.child_by_field_name("pattern") else {
                continue;
            };
            let name = self.text(pat).to_string();
            let ty = p
                .child_by_field_name("type")
                .map(|t| self.text(t).trim().to_string());
            self.sc.bind(&name, "param", None, None, ty);
        }
    }

    pub fn bind_struct_fields(&mut self, st: Node) {
        let Some(body) = st.child_by_field_name("body") else {
            return;
        };
        let kids: Vec<Node> = {
            let mut c = body.walk();
            body.named_children(&mut c).collect()
        };
        for f in kids {
            if f.kind() != "field_declaration" {
                continue;
            }
            let (Some(n), Some(t)) = (
                f.child_by_field_name("name"),
                f.child_by_field_name("type"),
            ) else {
                continue;
            };
            let name = self.text(n).to_string();
            let ty = simple_type_name(self.text(t));
            self.sc.bind(&name, "field", None, None, Some(ty));
        }
    }

    pub fn collect_let(&mut self, node: Node) {
        let Some(pat) = node.child_by_field_name("pattern") else {
            return;
        };
        if pat.kind() != "identifier" {
            return; // skip destructuring for now
        }
        let name = self.text(pat).to_string();
        let ty = node
            .child_by_field_name("type")
            .map(|t| simple_type_name(self.text(t)));
        self.sc.bind(&name, "local", None, None, ty);
    }
}
