//! Rust extraction via a tree-sitter AST walk.

pub mod calls;
pub mod helpers;
pub mod macros;
pub mod symbols;
pub mod types;
pub mod use_decl;

pub use helpers::*;

use tree_sitter::{Node, Parser};

use crate::analysis::ParsedFile;
use crate::analysis::scope::{self, ScopeStack};
use crate::cache::models::NewImport;

pub fn parse(source: &str) -> ParsedFile {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
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
        macro_depth: 0,
    };
    walker.walk(tree.root_node());
    walker.sc.finish_into(&mut walker.out);
    scope::resolve_locals(&mut walker.out);
    walker.out
}

pub(crate) struct Walker<'a> {
    pub(crate) src: &'a [u8],
    pub(crate) out: ParsedFile,
    /// Indices into `out.symbols` for the current lexical parent chain.
    pub(crate) stack: Vec<usize>,
    pub(crate) sc: ScopeStack,
    /// How many macro bodies deep this walk already is. A macro's arguments are
    /// re-parsed as a fragment and walked by a nested `Walker`, so this bounds
    /// `assert!(matches!(..))`-style nesting.
    pub(crate) macro_depth: u32,
}

impl<'a> Walker<'a> {
    pub fn walk(&mut self, node: Node) {
        match node.kind() {
            "mod_item" => {
                // `mod foo;` (no inline body) pulls in a sibling file.
                let has_body = node.child_by_field_name("body").is_some()
                    || child_of_kind(node, "declaration_list").is_some();
                if !has_body
                    && let Some(n) = node.child_by_field_name("name")
                {
                    let name = self.text(n).to_string();
                    let import_index = self.out.imports.len();
                    // `mod foo;` makes `foo` usable as a module path prefix.
                    self.sc
                        .bind(&name, "namespace", None, Some(import_index), None);
                    self.out.imports.push(NewImport {
                        raw_specifier: format!("self::{name}"),
                        imported_name: Some(name),
                        alias: None,
                        is_relative: true,
                        start_line: self.line(node),
                    });
                }
                self.enter_symbol(node, "module");
            }
            "function_item" => self.enter_symbol(node, "function"),
            "struct_item" | "union_item" => self.enter_symbol(node, "struct"),
            "enum_item" => self.enter_symbol(node, "enum"),
            "trait_item" => self.enter_symbol(node, "trait"),
            "impl_item" => self.enter_symbol(node, "impl"),
            "type_item" | "const_item" | "static_item" | "macro_definition" => {
                self.enter_symbol(node, "block");
            }
            "use_declaration" => self.collect_use(node),
            "let_declaration" => {
                self.collect_let(node);
                self.walk_children(node);
            }
            "call_expression" => {
                self.collect_call(node);
                self.walk_children(node);
            }
            "macro_invocation" => {
                self.collect_macro(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    /// Push a symbol, open a matching child scope, bind the symbol name in the
    /// parent scope, record params/fields, walk the body, then close up.
    pub fn enter_symbol(&mut self, node: Node, scope_kind: &str) {
        let idx = self.push_symbol(node);

        if let Some(i) = idx {
            let (name, kind) = {
                let s = &self.out.symbols[i];
                (s.name.clone(), s.kind.clone())
            };
            // A bare `foo()` can reach a free function or a tuple-struct ctor,
            // never a method or an `impl` block.
            if !matches!(kind.as_str(), "method" | "impl") {
                self.sc.bind(&name, "symbol", Some(i), None, None);
            }
            self.stack.push(i);
            self.sc
                .push(scope_kind, Some(i), node.start_byte() as i64, node.end_byte() as i64);

            match node.kind() {
                "function_item" => self.bind_signature(node),
                "struct_item" | "union_item" => self.bind_struct_fields(node),
                "enum_item" => self.collect_enum_variant_types(node),
                _ => {}
            }

            self.walk_children(node);

            self.sc.pop();
            self.stack.pop();
        } else {
            self.walk_children(node);
        }
    }

    pub fn walk_children(&mut self, node: Node) {
        let children: Vec<Node> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };
        for child in children {
            self.walk(child);
        }
    }

    pub fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.src).unwrap_or("")
    }

    pub fn line(&self, node: Node) -> i64 {
        node.start_position().row as i64 + 1
    }

    pub fn named(&self, node: Node) -> Option<String> {
        node.child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn has_pub(&self, node: Node) -> bool {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .any(|c| c.kind() == "visibility_modifier")
    }

    pub fn parent_is_impl_or_trait(&self) -> bool {
        self.stack.last().is_some_and(|&i| {
            matches!(self.out.symbols[i].kind.as_str(), "impl" | "trait")
        })
    }

    /// Simple name of the type the current `impl`/`trait` scope is for.
    pub fn enclosing_type_name(&self) -> Option<String> {
        self.stack.iter().rev().find_map(|&i| {
            let s = &self.out.symbols[i];
            match s.kind.as_str() {
                "impl" => Some(
                    s.name
                        .rsplit(" for ")
                        .next()
                        .unwrap_or(&s.name)
                        .trim()
                        .to_string(),
                ),
                "trait" => Some(s.name.clone()),
                _ => None,
            }
        })
    }

    pub fn param_count(&self, node: Node) -> Option<i64> {
        let params = node.child_by_field_name("parameters")?;
        let mut c = params.walk();
        let n = params
            .named_children(&mut c)
            .filter(|p| matches!(p.kind(), "parameter" | "self_parameter"))
            .count();
        Some(n as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_functions_structs_and_methods() {
        let src = r#"
pub struct Widget { size: u32 }

impl Widget {
    pub fn new() -> Self { Widget { size: 0 } }
    fn grow(&mut self) { self.size += 1; }
}

fn helper() {}

pub fn run() {
    let w = Widget::new();
    helper();
}
"#;
        let p = parse(src);
        assert!(p.parse_ok);

        let by_name = |n: &str| p.symbols.iter().find(|s| s.name == n);
        assert_eq!(by_name("Widget").unwrap().kind, "struct");
        assert!(by_name("Widget").unwrap().is_exported);
        assert_eq!(by_name("new").unwrap().kind, "method");
        assert_eq!(by_name("new").unwrap().type_name.as_deref(), Some("Widget"));
        assert_eq!(by_name("grow").unwrap().kind, "method");
        assert_eq!(by_name("helper").unwrap().kind, "function");
        assert!(!by_name("helper").unwrap().is_exported);
        assert_eq!(by_name("run").unwrap().kind, "function");

        // `new` is nested under the `impl Widget` symbol.
        let impl_idx = p.symbols.iter().position(|s| s.kind == "impl").unwrap();
        assert_eq!(by_name("new").unwrap().parent_index, Some(impl_idx));

        let call_names: Vec<&str> = p.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(call_names.contains(&"new")); // Widget::new()
        assert!(call_names.contains(&"helper"));

        // `helper()` inside `run` resolves in-file to the `helper` symbol.
        let helper_ref = p.refs.iter().find(|r| r.name == "helper").unwrap();
        assert!(helper_ref.resolved_local_symbol_index.is_some());
    }

    #[test]
    fn struct_fields_become_typed_bindings() {
        let src = r#"
struct Pool;
struct Server { pool: Pool, name: String }
"#;
        let p = parse(src);
        let field = p
            .bindings
            .iter()
            .find(|b| b.binding_kind == "field" && b.name == "pool")
            .expect("pool field binding");
        assert_eq!(field.type_expr.as_deref(), Some("Pool"));
    }

    #[test]
    fn flattens_use_declarations() {
        let src = r#"
use std::collections::{HashMap, HashSet};
use crate::cache::CacheDb;
use super::walker as w;
use anyhow::*;
"#;
        let p = parse(src);
        let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);

        assert!(spec("std::collections::HashMap").is_some());
        assert!(spec("std::collections::HashSet").is_some());
        let cc = spec("crate::cache::CacheDb").unwrap();
        assert!(cc.is_relative);
        assert_eq!(cc.imported_name.as_deref(), Some("CacheDb"));
        let sup = spec("super::walker").unwrap();
        assert_eq!(sup.alias.as_deref(), Some("w"));
        assert!(p.imports.iter().any(|i| i.raw_specifier == "anyhow"));

        // `w` is bound as an import; `anyhow::*` as a namespace.
        assert!(p
            .bindings
            .iter()
            .any(|b| b.binding_kind == "import" && b.name == "w"));
        assert!(p
            .bindings
            .iter()
            .any(|b| b.binding_kind == "namespace" && b.name == "anyhow"));
    }

    #[test]
    fn signature_types_become_type_refs() {
        let src = r#"
struct RunArgs;
enum CommandError {}

pub fn cmd_run(args: &RunArgs) -> Result<(), CommandError> { todo!() }
"#;
        let p = parse(src);
        let type_ref = |n: &str| {
            p.refs
                .iter()
                .find(|r| r.ref_kind == "type" && r.name == n)
        };

        // A struct used only as a parameter type is not dead code.
        let args = type_ref("RunArgs").expect("param type ref");
        assert!(!args.local_only);
        assert!(args.resolved_local_symbol_index.is_some());

        // The error type in the return position counts as a user too.
        assert!(type_ref("CommandError").is_some(), "return type ref");
    }

    #[test]
    fn type_refs_point_at_the_type_token() {
        // `--precise` sends `name_start_byte` to the language server, so it has
        // to land on `RunArgs`, not on the `&` or on the parameter name.
        let src = "struct RunArgs;\nfn f(args: &RunArgs) {}\n";
        let p = parse(src);
        let r = p
            .refs
            .iter()
            .find(|r| r.ref_kind == "type" && r.name == "RunArgs" && r.start_line == 2)
            .expect("param type ref");
        let at = r.name_start_byte.expect("name_start_byte") as usize;
        assert_eq!(&src[at..at + "RunArgs".len()], "RunArgs");
    }

    #[test]
    fn generic_parameters_are_not_mistaken_for_types() {
        // A project type named `T` must not collect edges from every `<T>`.
        let src = "struct T;\nfn identity<T>(value: T) -> T { value }\n";
        let p = parse(src);
        assert!(
            !p.refs.iter().any(|r| r.ref_kind == "type" && r.name == "T"),
            "generic placeholder T should not become a type ref"
        );
    }

    #[test]
    fn struct_fields_and_enum_payloads_become_type_refs() {
        let src = r#"
struct Pool;
struct RunArgs;
struct Server { pool: Pool }
struct Wrapper(Pool);
enum Command { Run(RunArgs) }
"#;
        let p = parse(src);
        let has = |n: &str| p.refs.iter().any(|r| r.ref_kind == "type" && r.name == n);

        assert!(has("Pool"), "named field type");
        assert!(has("RunArgs"), "enum variant payload type");

        // The field binding must survive alongside the new ref.
        assert!(
            p.bindings
                .iter()
                .any(|b| b.binding_kind == "field" && b.name == "pool"),
            "field binding regressed"
        );
    }

    #[test]
    fn a_local_does_not_shadow_a_type_of_the_same_name() {
        let src = r#"
struct Config;
fn load(cfg: &Config) {
    let Config = 1;
}
"#;
        let p = parse(src);
        let r = p
            .refs
            .iter()
            .find(|r| r.ref_kind == "type" && r.name == "Config")
            .expect("param type ref");
        assert!(!r.local_only, "type ref must skip local bindings");
        assert!(r.resolved_local_symbol_index.is_some());
    }

    /// Names of `call` refs found in `src`.
    fn call_names(src: &str) -> Vec<String> {
        parse(src)
            .refs
            .into_iter()
            .filter(|r| r.ref_kind == "call")
            .map(|r| r.name)
            .collect()
    }

    #[test]
    fn calls_inside_macro_arguments_are_found() {
        // The Jujutsu case: the only call to `run_inner` sits inside a
        // `try_join!`, whose arguments the grammar leaves as raw tokens.
        let src = r#"
async fn run_inner(x: u32) -> u32 { x }

async fn cmd_run() {
    futures::try_join!(
        async { run_inner(1).await },
        async { other().await },
    );
}

async fn other() -> u32 { 0 }
"#;
        let names = call_names(src);
        assert!(names.contains(&"run_inner".to_string()), "got {names:?}");
        assert!(names.contains(&"other".to_string()), "got {names:?}");

        // And it resolves in-file, which is what makes the edge appear.
        let p = parse(src);
        let r = p
            .refs
            .iter()
            .find(|r| r.name == "run_inner" && r.ref_kind == "call")
            .unwrap();
        assert!(!r.local_only);
        assert!(r.resolved_local_symbol_index.is_some());
    }

    #[test]
    fn a_macro_call_keeps_pointing_at_the_real_source_offsets() {
        // `--precise` sends `name_start_byte` to a language server, so an
        // offset rebased out of a fragment has to land on the real token.
        let src = "fn helper() {}\nfn f() {\n    assert!(helper() == 1);\n}\n";
        let p = parse(src);
        let r = p
            .refs
            .iter()
            .find(|r| r.name == "helper" && r.ref_kind == "call")
            .expect("call inside assert!");
        let at = r.name_start_byte.expect("name_start_byte") as usize;
        assert_eq!(&src[at..at + "helper".len()], "helper");
        assert_eq!(r.start_line, 3, "line must be the real source line");
    }

    #[test]
    fn block_bodied_macros_are_read_as_statements() {
        // `select!` does not hold a comma-separated expression list, so it
        // needs the second wrapper.
        let src = "fn f() {\n    tokio::select! {\n        a = poll() => {}\n    }\n}\n";
        let names = call_names(src);
        assert!(names.contains(&"poll".to_string()), "got {names:?}");
    }

    #[test]
    fn macros_nested_in_macros_are_followed() {
        let src = "fn g() -> u32 { 1 }\nfn f() {\n    assert!(matches!(g(), 1));\n}\n";
        let names = call_names(src);
        assert!(names.contains(&"g".to_string()), "got {names:?}");
    }

    #[test]
    fn a_macro_that_is_not_rust_is_skipped_rather_than_guessed() {
        // A DSL body must not spray invented references.
        let src = "fn f() {\n    weird_dsl! { < > < > @@ ## => => };\n}\n";
        let p = parse(src);
        assert!(p.parse_ok);
        // The macro itself is still recorded; nothing is invented from it.
        let names = call_names(src);
        assert!(names.contains(&"weird_dsl!".to_string()), "got {names:?}");
    }

    #[test]
    fn collects_string_literals_from_calls_and_macros() {
        let src = r#"
fn test_func() {
    let flag = matches.get_bool("verbose");
    println!("hello {}", "world_target");
}
"#;
        let p = parse(src);
        assert!(p.parse_ok);
        let vals: Vec<&str> = p.string_literals.iter().map(|s| s.value.as_str()).collect();
        assert!(vals.contains(&"verbose"), "should index 'verbose' call arg");
        assert!(vals.contains(&"world_target"), "should index 'world_target' macro arg");
    }
}
