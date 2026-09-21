//! Type-position references: the edges that tie a signature or a field to the
//! type it names.
//!
//! Without these a struct used only as a parameter type looks like dead code —
//! `impact` reports it with no callers at all, because the only thing the walk
//! recorded was the call graph.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::cache::models::NewRef;

use super::Walker;

impl<'a> Walker<'a> {
    /// Emit one `type` reference per nominal type named inside a type
    /// expression. `&mut Vec<Foo>` yields `Vec` and `Foo`; primitives,
    /// lifetimes and `skip` (the enclosing item's own generic parameters)
    /// yield nothing.
    ///
    /// `name_start_byte` points at the identifier token itself, because that is
    /// the offset `sync --precise` hands to the language server — pointed at
    /// the `&` or at a lifetime it would resolve a different token.
    pub fn collect_type_refs(&mut self, ty: Node, skip: &HashSet<String>) {
        if ty.kind() == "type_identifier" {
            let name = self.text(ty).to_string();
            if name.is_empty() || skip.contains(&name) {
                return;
            }
            self.out.refs.push(NewRef {
                name,
                ref_kind: "type".to_string(),
                receiver: None,
                start_line: self.line(ty),
                start_byte: ty.start_byte() as i64,
                name_start_byte: Some(ty.start_byte() as i64),
                receiver_kind: "none".to_string(),
                ..Default::default()
            });
            return;
        }
        // `Vec<Foo>`, `&mut T`, `dyn Trait`, `(A, B)` — descend and let every
        // nested identifier record itself. A `scoped_type_identifier`'s path
        // segments are plain `identifier` nodes, so only its `name` matches.
        let kids: Vec<Node> = {
            let mut c = ty.walk();
            ty.named_children(&mut c).collect()
        };
        for k in kids {
            self.collect_type_refs(k, skip);
        }
    }

    /// Names declared by an item's `<T, 'a, const N: usize>` list. They are
    /// placeholders, not types any file defines, so a reference to one must
    /// never become an edge to a same-named project type.
    pub fn generic_param_names(&self, node: Node) -> HashSet<String> {
        let mut out = HashSet::new();
        let Some(params) = node.child_by_field_name("type_parameters") else {
            return out;
        };
        let kids: Vec<Node> = {
            let mut c = params.walk();
            params.named_children(&mut c).collect()
        };
        for p in kids {
            // `<T>` is a `type_parameter` with a `name`; `<T: Clone>` a
            // `constrained_type_parameter` with a `left`. Lifetime and const
            // parameters carry no type identifier, so they fall through.
            let name = match p.kind() {
                "type_identifier" => Some(p),
                _ => p
                    .child_by_field_name("name")
                    .or_else(|| p.child_by_field_name("left")),
            };
            if let Some(n) = name.filter(|n| n.kind() == "type_identifier") {
                let text = self.text(n);
                if !text.is_empty() {
                    out.insert(text.to_string());
                }
            }
        }
        out
    }

    /// Types named by a struct or enum-variant body, covering both shapes:
    /// `{ a: Foo }` holds `field_declaration` children that carry a `type`,
    /// while the tuple form `(Foo)` holds the type nodes directly, each sitting
    /// in the list's own `type` field.
    ///
    /// `on_named_field` is handed every named field so the caller can also
    /// record its binding.
    pub fn collect_field_list_types(
        &mut self,
        list: Node,
        skip: &HashSet<String>,
        mut on_named_field: impl FnMut(&mut Self, Node, Node),
    ) {
        let kids: Vec<(Option<&str>, Node)> = {
            let mut c = list.walk();
            list.named_children(&mut c)
                .enumerate()
                .map(|(i, n)| (list.field_name_for_named_child(i as u32), n))
                .collect()
        };
        for (field_name, k) in kids {
            if field_name == Some("type") {
                self.collect_type_refs(k, skip); // tuple position: no name
                continue;
            }
            let Some(t) = k.child_by_field_name("type") else {
                continue;
            };
            if let Some(n) = k.child_by_field_name("name") {
                on_named_field(self, n, t);
            }
            self.collect_type_refs(t, skip);
        }
    }

    /// Types named by an enum's variant payloads: `Command::Run(RunArgs)` ties
    /// the enum to `RunArgs`, which is how a CLI arg struct reaches its router.
    pub fn collect_enum_variant_types(&mut self, en: Node) {
        let generics = self.generic_param_names(en);
        let Some(body) = en.child_by_field_name("body") else {
            return;
        };
        let variants: Vec<Node> = {
            let mut c = body.walk();
            body.named_children(&mut c).collect()
        };
        for variant in variants {
            // A unit variant (`Quiet`) has no body and names no type.
            let Some(payload) = variant.child_by_field_name("body") else {
                continue;
            };
            self.collect_field_list_types(payload, &generics, |_, _, _| {});
        }
    }
}
