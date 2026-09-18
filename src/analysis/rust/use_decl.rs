use tree_sitter::Node;

use crate::cache::models::NewImport;

use super::Walker;
use super::helpers::join;

pub struct UsePath {
    pub path: String,
    pub alias: Option<String>,
    pub wildcard: bool,
}

impl UsePath {
    pub fn plain(path: String) -> Self {
        Self {
            path,
            alias: None,
            wildcard: false,
        }
    }
}

impl<'a> Walker<'a> {
    pub fn collect_use(&mut self, node: Node) {
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
            let import_index = self.out.imports.len();
            // Bind the locally visible name so refs resolve to this import.
            let local = alias.clone().or_else(|| imported_name.clone());
            if wildcard {
                let ns = path.rsplit("::").next().unwrap_or(&path).to_string();
                self.sc.bind(&ns, "namespace", None, Some(import_index), None);
            } else if let Some(l) = &local {
                self.sc.bind(l, "import", None, Some(import_index), None);
            }
            self.out.imports.push(NewImport {
                raw_specifier: path,
                imported_name,
                alias,
                is_relative,
                start_line: line,
            });
        }
    }

    pub fn flatten_use(&self, node: Node, prefix: &str, out: &mut Vec<UsePath>) {
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
}
