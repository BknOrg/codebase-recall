//! Source-only C/C++ extraction. No preprocessing, compiler execution or type inference.
use super::{Language, ParsedFile, scope::ScopeStack, semantic::*};
use crate::cache::models::{NewImport, NewRef, NewSymbol};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

fn children(n: Node<'_>) -> Vec<Node<'_>> {
    n.named_children(&mut n.walk()).collect()
}
fn declarator_name(n: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        n.kind(),
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "namespace_identifier"
            | "operator_name"
            | "destructor_name"
            | "operator_cast"
    ) {
        return Some(n);
    }
    if n.kind() == "qualified_identifier" {
        return n.child_by_field_name("name").and_then(declarator_name);
    }
    if n.kind() == "parenthesized_declarator" {
        return children(n).into_iter().find_map(declarator_name);
    }
    n.child_by_field_name("declarator")
        .and_then(declarator_name)
        .or_else(|| {
            if n.kind().starts_with("template_") {
                n.child_by_field_name("name").and_then(declarator_name)
            } else {
                None
            }
        })
}

fn function_declarator(n: Node<'_>) -> Option<Node<'_>> {
    if n.kind() == "operator_cast" {
        return Some(n);
    }
    let mut at = declarator_name(n)?;
    loop {
        if at.kind() == "function_declarator" {
            return Some(at);
        }
        if matches!(
            at.kind(),
            "pointer_declarator" | "reference_declarator" | "array_declarator"
        ) {
            return None;
        }
        if at == n {
            return None;
        }
        at = at.parent()?;
    }
}
pub fn parse(source: &str, language: Language) -> ParsedFile {
    let grammar = if language == Language::C {
        tree_sitter_c::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return ParsedFile::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return ParsedFile::default();
    };
    let mut w = Walker {
        source,
        language,
        out: ParsedFile {
            parse_ok: !tree.root_node().has_error(),
            diagnostics: super::diagnostics(tree.root_node()),
            ..Default::default()
        },
        sc: ScopeStack::new(source.len() as i64),
        owners: Vec::new(),
        visibility: Visibility::Public,
        templates: Vec::new(),
        conditional: 0,
        linkage: None,
        visited: HashSet::new(),
        declarations: HashMap::new(),
    };
    w.walk(tree.root_node());
    w.sc.finish_into(&mut w.out);
    super::scope::resolve_locals_for(&mut w.out, language.group());
    w.out
}
struct Walker<'a> {
    source: &'a str,
    language: Language,
    out: ParsedFile,
    sc: ScopeStack,
    owners: Vec<usize>,
    visibility: Visibility,
    templates: Vec<String>,
    conditional: usize,
    linkage: Option<String>,
    visited: HashSet<usize>,
    declarations: HashMap<(usize, usize, String, String), usize>,
}
impl Walker<'_> {
    fn text(&self, n: Node<'_>) -> &str {
        self.source.get(n.byte_range()).unwrap_or("")
    }
    fn owner(&self) -> Option<usize> {
        self.owners.last().copied()
    }
    fn in_function(&self) -> bool {
        self.owners
            .iter()
            .any(|&i| self.out.symbols[i].kind.callable())
    }
    fn in_type(&self) -> bool {
        self.owner()
            .is_some_and(|i| self.out.symbols[i].kind.is_type())
    }
    fn walk_children(&mut self, n: Node<'_>) {
        for c in children(n) {
            self.walk(c);
        }
    }
    fn bind(
        &mut self,
        name: &str,
        kind: &str,
        symbol: Option<usize>,
        ty: Option<String>,
        at: usize,
    ) {
        self.sc.bind(name, kind, symbol, None, ty);
        if let Some(b) = self.sc.bindings.last_mut() {
            b.declaration_byte = Some(at as i64);
        }
    }
    fn add(
        &mut self,
        n: Node<'_>,
        name_node: Option<Node<'_>>,
        name: String,
        kind: SymbolKind,
    ) -> usize {
        let identity = (n.start_byte(), n.end_byte(), kind.to_string(), name.clone());
        if let Some(&index) = self.declarations.get(&identity) {
            return index;
        }
        let parent_index = self.owner();
        let owner = parent_index.map(|i| self.out.symbols[i].semantic.qualified_name.clone());
        let qualified_name = owner
            .as_ref()
            .map_or_else(|| name.clone(), |p| format!("{p}::{name}"));
        let mut semantic = SymbolFacts {
            kind,
            role: SymbolRole::Definition,
            scope: if self.in_type() {
                ScopeKind::Type
            } else if self.in_function() {
                ScopeKind::Function
            } else {
                ScopeKind::Module
            },
            qualified_name,
            owner,
            name_range: name_node.map(|n| [n.start_byte(), n.end_byte()]),
            visibility: self.visibility,
            type_parameters: self.templates.clone(),
            ..Default::default()
        };
        semantic
            .language_details
            .insert("language".into(), self.language.group().into());
        if let Some(linkage) = &self.linkage {
            semantic
                .language_details
                .insert("linkage".into(), linkage.clone().into());
        }
        if self.owners.iter().any(|&i| {
            self.out.symbols[i]
                .semantic
                .language_details
                .get("internal_linkage")
                .and_then(|v| v.as_bool())
                == Some(true)
        }) {
            semantic
                .language_details
                .insert("internal_linkage".into(), true.into());
        }
        if self.conditional > 0 {
            semantic
                .language_details
                .insert("conditional".into(), true.into());
        }
        let idx = self.out.symbols.len();
        self.declarations.insert(identity, idx);
        self.out.symbols.push(NewSymbol {
            name: name.clone(),
            kind,
            parent_index,
            is_exported: self.visibility == Visibility::Public,
            start_line: n.start_position().row as i64 + 1,
            end_line: n.end_position().row as i64 + 1,
            start_byte: n.start_byte() as i64,
            end_byte: n.end_byte() as i64,
            signature: super::node_signature(n, self.source.as_bytes()),
            semantic,
            ..Default::default()
        });
        self.bind(
            &name,
            if kind == SymbolKind::Field {
                "field"
            } else if self.language == Language::C
                && matches!(
                    kind,
                    SymbolKind::Struct | SymbolKind::Union | SymbolKind::Enum
                )
            {
                "tag"
            } else {
                "symbol"
            },
            Some(idx),
            None,
            name_node.unwrap_or(n).start_byte(),
        );
        idx
    }
    fn enter(&mut self, n: Node<'_>, idx: usize, scope: &str) {
        self.owners.push(idx);
        self.sc
            .push(scope, Some(idx), n.start_byte() as i64, n.end_byte() as i64);
        for parameter in self.templates.clone() {
            let left = parameter.split('=').next().unwrap_or("");
            if let Some(name) = left
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .rfind(|v| !v.is_empty())
            {
                self.bind(name, "type_param", None, None, n.start_byte());
            }
        }
    }
    fn leave(&mut self) {
        self.sc.pop();
        self.owners.pop();
    }
    fn walk(&mut self, n: Node<'_>) {
        // A type node can be reached both as a declaration type and a recovered
        // function return type. Its source occurrence must only be extracted once.
        if !self.visited.insert(n.id()) {
            return;
        }
        match n.kind() {
            "linkage_specification" => {
                let old = self.linkage.clone();
                self.linkage = n
                    .child_by_field_name("value")
                    .map(|v| self.text(v).trim_matches('"').to_owned());
                self.walk_children(n);
                self.linkage = old;
            }
            "namespace_alias_definition" | "using_declaration" => {
                self.out.diagnostics.push(serde_json::json!({"kind":"unsupported_construct","construct":n.kind(),"start_byte":n.start_byte(),"message":"namespace alias/using lookup requires compiler evidence"}));
            }
            "namespace_definition" => {
                let name_node = n.child_by_field_name("name");
                let name = name_node.map_or_else(
                    || format!("<anonymous@{}>", n.start_byte()),
                    |v| self.text(v).to_owned(),
                );
                let idx = self.add(n, name_node, name, SymbolKind::Namespace);
                if name_node.is_none() {
                    self.out.symbols[idx]
                        .semantic
                        .language_details
                        .insert("internal_linkage".into(), true.into());
                }
                self.enter(n, idx, "namespace");
                self.walk_children(n);
                self.leave();
            }
            "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                self.aggregate(n)
            }
            "function_definition" => {
                if let Some(d) = n.child_by_field_name("declarator") {
                    self.function(n, d, true);
                }
            }
            "declaration" | "field_declaration" | "type_definition" => self.declaration(n),
            "alias_declaration" | "concept_definition" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let kind = if n.kind() == "concept_definition" {
                        SymbolKind::Concept
                    } else {
                        SymbolKind::TypeAlias
                    };
                    let idx = self.add(n, Some(name), self.text(name).to_owned(), kind);
                    self.out.symbols[idx].semantic.declared_type = n
                        .child_by_field_name("type")
                        .map(|v| self.text(v).to_owned());
                }
            }
            "template_declaration" => {
                let old = self.templates.clone();
                self.templates = n
                    .child_by_field_name("parameters")
                    .map(|p| {
                        children(p)
                            .into_iter()
                            .map(|c| self.text(c).to_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                self.walk_children(n);
                self.templates = old;
            }
            "access_specifier" => {
                self.visibility = match self.text(n) {
                    "private" => Visibility::Private,
                    "protected" => Visibility::Protected,
                    _ => Visibility::Public,
                }
            }
            "preproc_include" => {
                if let Some(path) = n.child_by_field_name("path") {
                    if matches!(path.kind(), "string_literal" | "system_lib_string") {
                        self.out.imports.push(NewImport {
                            raw_specifier: self.text(path).to_owned(),
                            is_relative: path.kind() == "string_literal",
                            start_line: n.start_position().row as i64 + 1,
                            ..Default::default()
                        });
                    } else {
                        self.out.diagnostics.push(serde_json::json!({"kind":"macro_include","start_byte":n.start_byte(),"message":"include depends on preprocessing; target unknown"}));
                    }
                }
            }
            "preproc_def" | "preproc_function_def" => {
                if let Some(name) = n.child_by_field_name("name") {
                    self.add(n, Some(name), self.text(name).to_owned(), SymbolKind::Macro);
                }
            }
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_else" => {
                self.conditional += 1;
                self.walk_children(n);
                self.conditional -= 1;
            }
            "compound_statement" | "for_statement" | "for_range_loop" => {
                self.sc.push(
                    "block",
                    self.owner(),
                    n.start_byte() as i64,
                    n.end_byte() as i64,
                );
                self.walk_children(n);
                self.sc.pop();
            }
            "new_expression" => {
                if let Some(t) = n.child_by_field_name("type") {
                    let name = declarator_name(t).unwrap_or(t);
                    self.reference(
                        name,
                        "call",
                        None,
                        Some(
                            n.child_by_field_name("arguments")
                                .map(children)
                                .map_or(0, |a| a.len()) as i64,
                        ),
                        "none",
                    );
                }
                if let Some(a) = n.child_by_field_name("arguments") {
                    self.walk_children(a);
                }
            }
            "call_expression" => {
                self.call(n);
                for c in children(n) {
                    if Some(c) != n.child_by_field_name("function") {
                        self.walk(c);
                    }
                }
            }
            "field_expression" => {
                self.member(n, "read", None);
            }
            "identifier" => {
                if self.in_function() {
                    self.reference(n, "read", None, None, "none");
                }
            }
            "type_identifier" => {
                let tag = self.language == Language::C
                    && n.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            "struct_specifier" | "union_specifier" | "enum_specifier"
                        )
                    });
                self.reference(n, if tag { "tag" } else { "type" }, None, None, "none");
            }
            _ => self.walk_children(n),
        }
    }
    fn aggregate(&mut self, n: Node<'_>) {
        let name_node = n.child_by_field_name("name");
        let kind = match n.kind() {
            "class_specifier" => SymbolKind::Class,
            "struct_specifier" => SymbolKind::Struct,
            "union_specifier" => SymbolKind::Union,
            _ => SymbolKind::Enum,
        };
        if n.child_by_field_name("body").is_none()
            && self.language == Language::C
            && n.parent().is_some_and(|p| {
                p.child_by_field_name("declarator").is_some()
                    || matches!(p.kind(), "parameter_declaration" | "type_descriptor")
            })
        {
            if let Some(name) = name_node {
                self.reference(name, "tag", None, None, "none");
            }
            return;
        }
        let name = name_node.map_or_else(
            || format!("<anonymous@{}>", n.start_byte()),
            |v| self.text(v).to_owned(),
        );
        let idx = self.add(n, name_node, name, kind);
        let body = n.child_by_field_name("body");
        if body.is_none() {
            self.out.symbols[idx].semantic.role = SymbolRole::Declaration;
        }
        for clause in children(n)
            .into_iter()
            .filter(|c| c.kind() == "base_class_clause")
        {
            for base in children(clause).into_iter().filter(|c| {
                matches!(
                    c.kind(),
                    "type_identifier" | "qualified_identifier" | "template_type"
                )
            }) {
                let target = self.text(base).to_owned();
                self.out.symbols[idx].semantic.relations.push(TypeRelation {
                    kind: RelationKind::Inherits,
                    target,
                });
            }
        }
        let old = self.visibility;
        self.visibility = if kind == SymbolKind::Class {
            Visibility::Private
        } else {
            Visibility::Public
        };
        self.enter(n, idx, "type");
        if let Some(body) = body {
            for c in children(body) {
                if c.kind() == "enumerator" {
                    if let Some(name) = c.child_by_field_name("name") {
                        self.add(
                            c,
                            Some(name),
                            self.text(name).to_owned(),
                            SymbolKind::EnumMember,
                        );
                    }
                } else {
                    self.walk(c);
                }
            }
        }
        self.leave();
        self.visibility = old;
    }
    fn declaration(&mut self, n: Node<'_>) {
        let ty = n.child_by_field_name("type");
        if let Some(t) = ty
            && matches!(
                t.kind(),
                "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier"
            )
        {
            self.walk(t);
        }
        let ds: Vec<_> = n
            .children_by_field_name("declarator", &mut n.walk())
            .collect();
        for d in ds {
            if function_declarator(d).is_some() && n.kind() != "type_definition" {
                self.function(n, d, false);
                continue;
            }
            let Some(name_node) = declarator_name(d) else {
                continue;
            };
            let name = self.text(name_node).to_owned();
            let ty_text = ty.map(|t| self.text(t).to_owned());
            let raw = self.text(n).to_owned();
            let kind = if n.kind() == "type_definition" {
                SymbolKind::TypeAlias
            } else if self.in_type() {
                SymbolKind::Field
            } else if raw
                .split_whitespace()
                .any(|w| matches!(w, "constexpr" | "const"))
            {
                SymbolKind::Constant
            } else {
                SymbolKind::Variable
            };
            if self.in_function() && kind != SymbolKind::TypeAlias {
                self.bind(&name, "local", None, ty_text, name_node.start_byte());
            } else {
                let idx = self.add(d, Some(name_node), name, kind);
                let suffix = self.text(d).replacen(self.text(name_node), "", 1);
                let suffix = suffix.split('=').next().unwrap_or("").trim().to_owned();
                let facts = &mut self.out.symbols[idx].semantic;
                facts.declared_type = ty_text.map(|t| format!("{t} {suffix}").trim().to_owned());
                if let Some(b) = self.sc.bindings.last_mut() {
                    b.type_expr = facts.declared_type.clone();
                }
                facts.modifiers.is_static = Some(raw.split_whitespace().any(|w| w == "static"));
                facts.modifiers.is_mutable = Some(
                    !raw.split_whitespace()
                        .any(|w| matches!(w, "const" | "constexpr")),
                );
                if facts.modifiers.is_static == Some(true) && !self.in_type() {
                    self.out.symbols[idx]
                        .semantic
                        .language_details
                        .insert("internal_linkage".into(), true.into());
                }
            }
            if let Some(v) = d.child_by_field_name("value") {
                self.walk(v);
            }
        }
        if let Some(t) = ty
            && !matches!(
                t.kind(),
                "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier"
            )
        {
            self.walk(t);
        }
    }
    fn function(&mut self, n: Node<'_>, d: Node<'_>, definition: bool) {
        let Some(fd) = function_declarator(d) else {
            return;
        };
        let Some(name_node) = declarator_name(fd) else {
            return;
        };
        let name = if name_node.kind() == "operator_cast" {
            self.text(name_node)
                .split('(')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned()
        } else {
            self.text(name_node).to_owned()
        };
        let declarator_text = self
            .text(fd.child_by_field_name("declarator").unwrap_or(name_node))
            .to_owned();
        let explicit_owner = declarator_text.rsplit_once("::").map(|(o, _)| o.to_owned());
        let owner_name = explicit_owner
            .as_deref()
            .or_else(|| self.owner().map(|i| self.out.symbols[i].name.as_str()));
        let kind = if name.starts_with('~') {
            SymbolKind::Destructor
        } else if owner_name.is_some_and(|o| o.rsplit("::").next() == Some(name.as_str())) {
            SymbolKind::Constructor
        } else if self.in_type() || explicit_owner.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let idx = self.add(if definition { n } else { d }, Some(name_node), name, kind);
        self.out.symbols[idx].signature = super::node_signature(n, self.source.as_bytes());
        let declarator = self.text(d).to_owned();
        self.out.symbols[idx]
            .semantic
            .language_details
            .insert("declarator".into(), declarator.into());
        let raw = self.text(n).split('{').next().unwrap_or("").to_owned();
        let return_type = n
            .child_by_field_name("type")
            .map(|t| self.text(t).to_owned());
        let param_owner = if fd.kind() == "operator_cast" {
            fd.child_by_field_name("declarator").unwrap_or(fd)
        } else {
            fd
        };
        let params = param_owner
            .child_by_field_name("parameters")
            .map(children)
            .unwrap_or_default();
        let mut ordered = Vec::new();
        for p in &params {
            if self.text(*p).trim() == "void" {
                continue;
            }
            let pn = p
                .child_by_field_name("declarator")
                .and_then(declarator_name);
            ordered.push(Parameter {
                name: pn.map(|v| self.text(v).to_owned()),
                declared_type: Some(
                    pn.map_or_else(
                        || self.text(*p).to_owned(),
                        |v| self.text(*p).replacen(self.text(v), "", 1),
                    )
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
                ),
                optional: p.child_by_field_name("default_value").is_some(),
                variadic: self.text(*p).contains("..."),
                receiver: self.text(*p).trim_start().starts_with("this "),
            });
        }
        // C's f() has unknown arity, unlike C++ f().
        let defaults: Vec<_> = params
            .iter()
            .map(|p| {
                p.child_by_field_name("default_value")
                    .map(|v| self.text(v).to_owned())
            })
            .collect();
        let qualifier = children(fd)
            .into_iter()
            .find(|v| v.kind() == "ref_qualifier")
            .map(|v| self.text(v).to_owned());
        let unknown_arity = self.language == Language::C && params.is_empty();
        let symbol_name = self.out.symbols[idx].name.clone();
        let facts = &mut self.out.symbols[idx].semantic;
        facts.role = if definition {
            SymbolRole::Definition
        } else {
            SymbolRole::Declaration
        };
        facts.parameters = (!unknown_arity).then_some(ordered);
        facts
            .language_details
            .insert("parameter_defaults".into(), serde_json::json!(defaults));
        if let Some(q) = qualifier {
            facts
                .language_details
                .insert("ref_qualifier".into(), q.into());
        }
        facts.return_type = return_type;
        facts.modifiers.is_static = Some(raw.split_whitespace().any(|w| w == "static"));
        facts.modifiers.is_virtual = raw
            .split_whitespace()
            .any(|w| matches!(w, "virtual" | "override"))
            .then_some(true);
        if let Some(owner) = explicit_owner {
            let owner = if let Some(prefix) = &facts.owner {
                if !owner.starts_with(&format!("{prefix}::")) {
                    format!("{prefix}::{owner}")
                } else {
                    owner
                }
            } else {
                owner
            };
            facts.owner = Some(owner.clone());
            facts.qualified_name = format!("{owner}::{symbol_name}");
        }
        self.out.symbols[idx].param_count = if unknown_arity {
            None
        } else {
            Some(params.len() as i64)
        };
        if raw.contains("static ") && !self.in_type() {
            self.out.symbols[idx]
                .semantic
                .language_details
                .insert("internal_linkage".into(), true.into());
        }
        for qualifier in [
            "const",
            "volatile",
            "noexcept",
            "constexpr",
            "consteval",
            "extern",
            "inline",
            "thread_local",
            "override",
            "final",
        ] {
            if raw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|w| w == qualifier)
            {
                self.out.symbols[idx]
                    .semantic
                    .language_details
                    .insert(qualifier.into(), true.into());
            }
        }
        self.enter(n, idx, "function");
        for p in params {
            if let Some(pn) = p
                .child_by_field_name("declarator")
                .and_then(declarator_name)
            {
                self.bind(
                    self.text(pn).to_owned().as_str(),
                    "param",
                    None,
                    p.child_by_field_name("type")
                        .map(|t| self.text(t).to_owned()),
                    pn.start_byte(),
                );
            }
            if let Some(t) = p.child_by_field_name("type") {
                self.walk(t);
            }
            if let Some(v) = p.child_by_field_name("default_value") {
                self.walk(v);
            }
        }
        if let Some(t) = n.child_by_field_name("type") {
            self.walk(t);
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.walk(body);
        }
        self.leave();
    }
    fn reference(
        &mut self,
        n: Node<'_>,
        kind: &str,
        receiver: Option<String>,
        arity: Option<i64>,
        rk: &str,
    ) {
        if self.text(n).is_empty() {
            return;
        }
        self.out.refs.push(NewRef {
            name: self.text(n).to_owned(),
            ref_kind: kind.into(),
            receiver,
            start_line: n.start_position().row as i64 + 1,
            start_byte: n.start_byte() as i64,
            name_start_byte: Some(n.start_byte() as i64),
            arg_count: arity,
            receiver_kind: rk.into(),
            ..Default::default()
        });
    }
    fn member(&mut self, n: Node<'_>, kind: &str, arity: Option<i64>) {
        if let Some(field) = n.child_by_field_name("field") {
            let receiver = n
                .child_by_field_name("argument")
                .map(|v| self.text(v).to_owned());
            let rk = if receiver.as_deref() == Some("this") {
                "self"
            } else {
                "value"
            };
            self.reference(field, kind, receiver, arity, rk);
        }
    }
    fn call(&mut self, n: Node<'_>) {
        let Some(f) = n.child_by_field_name("function") else {
            return;
        };
        let arity = n
            .child_by_field_name("arguments")
            .map(|a| children(a).len() as i64);
        if f.kind() == "field_expression" {
            self.member(f, "call", arity);
        } else if f.kind() == "qualified_identifier" {
            if let Some(name) = f.child_by_field_name("name").and_then(declarator_name) {
                let receiver = self.text(f).rsplit_once("::").map(|(v, _)| v.to_owned());
                self.reference(name, "call", receiver, arity, "path");
            }
        } else if let Some(name) = declarator_name(f) {
            self.reference(name, "call", None, arity, "none");
        } else {
            self.out.diagnostics.push(serde_json::json!({"kind":"indirect_call","start_byte":n.start_byte(),"message":"computed callable requires compiler evidence"}));
        }
        let hints = n.child_by_field_name("arguments").map(|a| {
            children(a)
                .into_iter()
                .map(|v| match v.kind() {
                    "number_literal" => Some(
                        if self.text(v).contains(['.', 'e', 'E']) {
                            "double"
                        } else {
                            "int"
                        }
                        .to_owned(),
                    ),
                    "string_literal" => Some("const char *".into()),
                    "char_literal" => Some("char".into()),
                    "true" | "false" => Some("bool".into()),
                    _ => None,
                })
                .collect()
        });
        if let Some(r) = self.out.refs.last_mut() {
            r.argument_types = hints;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn symbol<'a>(p: &'a ParsedFile, name: &str) -> &'a NewSymbol {
        p.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}; got {:?}",
                    p.symbols
                        .iter()
                        .map(|s| (&s.name, &s.kind))
                        .collect::<Vec<_>>()
                )
            })
    }
    #[test]
    fn c_declarators_bindings_and_unicode() {
        let src = "// 日本語\ntypedef struct Point { int x, y; } Point; static int hidden(int x) { int local=x; return local; } int (*callback)(int); int array[4]; enum Mode { ON, OFF }; int sum(int first, ...) { return hidden(first); }";
        let p = parse(src, Language::C);
        assert!(p.parse_ok, "{:?}", p.diagnostics);
        assert_eq!(symbol(&p, "Point").kind, SymbolKind::Struct);
        assert_eq!(p.symbols.iter().filter(|s| s.name == "Point").count(), 2);
        assert_eq!(symbol(&p, "callback").kind, SymbolKind::Variable);
        assert!(
            symbol(&p, "array")
                .semantic
                .declared_type
                .as_ref()
                .unwrap()
                .contains("[4]")
        );
        assert_eq!(symbol(&p, "x").kind, SymbolKind::Field);
        assert_eq!(symbol(&p, "OFF").kind, SymbolKind::EnumMember);
        assert!(
            symbol(&p, "sum")
                .semantic
                .parameters
                .as_ref()
                .unwrap()
                .iter()
                .any(|p| p.variadic)
        );
        assert!(!p.symbols.iter().any(|s| s.name == "local"));
        assert!(p.refs.iter().any(|r| r.name == "local" && r.local_only));
        let range = symbol(&p, "hidden").semantic.name_range.unwrap();
        assert_eq!(&src[range[0]..range[1]], "hidden");
    }
    #[test]
    fn cpp_members_templates_and_ownership() {
        let src = "namespace app { template<class T> concept Number = requires(T v) { v + v; }; struct Base { virtual int run(int x=1); }; class Child : public Base { public: Child(); ~Child(); static int count; int run(int x=2) override { return count; } }; Child::Child() {} Child::~Child() {} }";
        let p = parse(src, Language::Cpp);
        assert!(p.parse_ok, "{:?}", p.diagnostics);
        assert_eq!(symbol(&p, "Number").kind, SymbolKind::Concept);
        assert_eq!(symbol(&p, "Child").semantic.qualified_name, "app::Child");
        assert!(p.symbols.iter().any(|s| s.kind == SymbolKind::Constructor));
        assert!(p.symbols.iter().any(|s| s.kind == SymbolKind::Destructor));
        assert_eq!(symbol(&p, "count").kind, SymbolKind::Field);
        assert_eq!(symbol(&p, "count").semantic.visibility, Visibility::Public);
        assert!(
            symbol(&p, "Child")
                .semantic
                .relations
                .iter()
                .any(|r| r.target == "Base")
        );
        assert!(
            p.symbols
                .iter()
                .filter(|s| s.name == "run")
                .all(|s| s.semantic.accepts_arity(0))
        );
    }
    #[test]
    fn c23_and_cpp23_partial_syntax_is_explicit() {
        for (lang, src) in [
            (
                Language::C,
                "constexpr int limit = 4; static_assert(1); int main(void) { auto value = 2; return value; }",
            ),
            (
                Language::Cpp,
                "struct Box { constexpr int get(this Box const& self) { if consteval { return 1; } else { return 2; } } };",
            ),
        ] {
            let p = parse(src, lang);
            assert!(!p.symbols.is_empty());
            assert!(p.parse_ok || !p.diagnostics.is_empty());
        }
    }
    #[test]
    fn pointer_precedence_operator_conversion_and_comma_overload_ids() {
        let p = parse(
            "int (*callback)(int); int *factory(); void run(), run(int); struct Flag { operator bool() const { return true; } };",
            Language::Cpp,
        );
        assert!(p.parse_ok, "{:?}", p.diagnostics);
        assert_eq!(symbol(&p, "callback").kind, SymbolKind::Variable);
        assert_eq!(symbol(&p, "factory").kind, SymbolKind::Function);
        assert!(
            p.symbols
                .iter()
                .any(|s| s.name.starts_with("operator bool") && s.kind == SymbolKind::Method)
        );
        let runs: Vec<_> = p.symbols.iter().filter(|s| s.name == "run").collect();
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[0].start_byte, runs[1].start_byte);
    }
    #[test]
    fn recovered_header_types_are_not_extracted_twice() {
        let source = include_str!("../../tests/fixtures/header_duplicate_app/Event.h");
        for language in [Language::C, Language::Cpp] {
            let parsed = parse(source, language);
            let mut ids = std::collections::HashSet::new();
            for symbol in &parsed.symbols {
                assert!(
                    ids.insert((
                        symbol.kind.to_string(),
                        symbol.name.clone(),
                        symbol.start_byte,
                        symbol.end_byte
                    )),
                    "duplicate {symbol:?}"
                );
            }
            assert_eq!(
                parsed
                    .symbols
                    .iter()
                    .filter(|s| s.kind == SymbolKind::Enum && s.name == "Type")
                    .count(),
                1
            );
            for name in ["Invalid", "Timer", "DeferredInvoke"] {
                assert_eq!(
                    parsed
                        .symbols
                        .iter()
                        .filter(|s| s.kind == SymbolKind::EnumMember && s.name == name)
                        .count(),
                    1
                );
            }
            let mut bindings = std::collections::HashSet::new();
            for b in &parsed.bindings {
                if let Some(symbol) = b.symbol_index {
                    assert!(
                        bindings.insert((symbol, b.name.clone())),
                        "duplicate binding {b:?}"
                    );
                }
            }
            if language == Language::Cpp {
                assert!(parsed.parse_ok);
                let ty = parsed.symbols.iter().find(|s| s.name == "Type").unwrap();
                assert_eq!(ty.semantic.owner.as_deref(), Some("Core::Event"));
            } else {
                assert!(!parsed.parse_ok);
            }
        }
        let parsed = parse(
            "int same(int value); int same(double value); enum A { First }; enum B { Second };",
            Language::Cpp,
        );
        assert_eq!(
            parsed.symbols.iter().filter(|s| s.name == "same").count(),
            2
        );
    }
}
