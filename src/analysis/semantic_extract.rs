//! Enrich all analyzer outputs from AST fields, without interpreting arbitrary expressions.
use super::{Language, ParsedFile, semantic::*};
use crate::cache::models::NewSymbol;
use tree_sitter::Node;

fn text<'a>(n: Node<'_>, src: &'a str) -> &'a str {
    src.get(n.byte_range()).unwrap_or("")
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    n.named_children(&mut n.walk()).collect()
}
fn child<'a>(n: Node<'a>, kind: &str) -> Option<Node<'a>> {
    children(n).into_iter().find(|n| n.kind() == kind)
}
fn ancestor_name(mut n: Node<'_>) -> Option<Node<'_>> {
    while let Some(p) = n.parent() {
        if matches!(
            p.kind(),
            "class_declaration" | "object_declaration" | "class_definition"
        ) {
            return name(p);
        }
        n = p;
    }
    None
}
fn field_text(n: Node<'_>, field: &str, src: &str) -> Option<String> {
    n.child_by_field_name(field)
        .map(|n| text(n, src).to_string())
}
fn name(n: Node<'_>) -> Option<Node<'_>> {
    n.child_by_field_name("name").or_else(|| {
        children(n).into_iter().find(|n| {
            matches!(
                n.kind(),
                "identifier" | "type_identifier" | "field_identifier" | "simple_identifier"
            )
        })
    })
}
pub fn tree(language: Language, source: &str) -> Option<tree_sitter::Tree> {
    let grammar: tree_sitter::Language = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).ok()?;
    if language == Language::Kotlin {
        parser.parse(super::kotlin_comments::mask(source), None)
    } else {
        parser.parse(source, None)
    }
}

pub fn enrich(language: Language, src: &str, out: &mut ParsedFile) {
    if language == Language::Toml {
        return;
    }
    if let Some(tree) = tree(language, src) {
        let root = tree.root_node();
        let package = children(root)
            .into_iter()
            .find(|n| {
                matches!(
                    n.kind(),
                    "package_declaration" | "package_header" | "package_clause"
                )
            })
            .map(|n| {
                text(n, src)
                    .trim()
                    .trim_start_matches("package")
                    .trim()
                    .trim_end_matches(';')
                    .to_string()
            });
        visit(root, language, src, out);
        bind_declarations(root, language, src, out);
        member_references(root, language, src, out);
        argument_types(root, language, src, out);
        // Parent recovery also covers new fields, constructors and enum members.
        for i in 0..out.symbols.len() {
            {
                let s = &out.symbols[i];
                let parent = out
                    .symbols
                    .iter()
                    .enumerate()
                    .filter(|(j, p)| {
                        *j != i
                            && p.start_byte <= s.start_byte
                            && p.end_byte >= s.end_byte
                            && (p.start_byte < s.start_byte || p.end_byte > s.end_byte)
                            && !matches!(p.kind.as_str(), "variable" | "field" | "property")
                    })
                    .min_by_key(|(_, p)| p.end_byte - p.start_byte)
                    .map(|(j, _)| j);
                if parent.is_some() {
                    out.symbols[i].parent_index = parent;
                }
                // Fields declared in constructors belong to the enclosing type.
                if matches!(
                    out.symbols[i].semantic.kind,
                    SymbolKind::Field | SymbolKind::Property
                ) {
                    let mut owner = out.symbols[i].parent_index;
                    while let Some(p) = owner {
                        if out.symbols[p].semantic.kind.is_type() {
                            break;
                        }
                        owner = out.symbols[p].parent_index;
                    }
                    if owner.is_some() {
                        out.symbols[i].parent_index = owner;
                    }
                }
            }
            out.symbols[i].semantic.package = package.clone();
        }
    }
    for s in &mut out.symbols {
        if s.semantic.kind == SymbolKind::Unknown {
            s.semantic.kind = SymbolKind::from_legacy(&s.kind);
        }
    }
    let embedded: Vec<_> = out
        .symbols
        .iter()
        .filter(|s| {
            s.semantic.language_details.get("embedded") == Some(&serde_json::Value::Bool(true))
        })
        .filter_map(|s| Some((s.parent_index?, s.semantic.declared_type.clone()?)))
        .collect();
    for (owner, target) in embedded {
        out.symbols[owner].semantic.relations.push(TypeRelation {
            kind: RelationKind::Embeds,
            target,
        });
    }
    // Compute qualified ownership without depending on extraction order.
    for i in 0..out.symbols.len() {
        let mut path = vec![out.symbols[i].name.clone()];
        let mut parent = out.symbols[i].parent_index;
        let mut seen = std::collections::HashSet::from([i]);
        while let Some(p) = parent.filter(|p| *p < out.symbols.len() && seen.insert(*p)) {
            path.push(out.symbols[p].name.clone());
            parent = out.symbols[p].parent_index;
        }
        if let Some(pkg) = &out.symbols[i].semantic.package {
            path.push(pkg.replace('.', "::"));
        }
        path.reverse();
        let owner = (path.len() > 1).then(|| path[..path.len() - 1].join("::"));
        out.symbols[i].semantic.owner = owner;
        out.symbols[i].semantic.qualified_name = path.join("::");
        let parent = out.symbols[i].parent_index.and_then(|p| out.symbols.get(p));
        let scope = match parent.map(|p| p.semantic.kind) {
            Some(k) if k.is_type() || k == SymbolKind::Impl => ScopeKind::Type,
            Some(k) if k.callable() => ScopeKind::Function,
            Some(_) => ScopeKind::Namespace,
            None => ScopeKind::Module,
        };
        out.symbols[i].semantic.scope = scope;
    }
    for symbol in &mut out.symbols {
        symbol.kind = symbol.semantic.kind;
    }
    for binding in &mut out.bindings {
        if binding.binding_kind == "field" {
            let owner = out
                .scopes
                .get(binding.scope_index)
                .and_then(|s| s.owner_symbol_index);
            binding.symbol_index = out.symbols.iter().position(|s| {
                s.name == binding.name
                    && s.parent_index == owner
                    && matches!(s.kind, SymbolKind::Field | SymbolKind::Property)
            });
        }
        if binding.scope_index == 0
            && binding.binding_kind == "local"
            && language == Language::Go
            && let Some(si) = out
                .symbols
                .iter()
                .position(|s| s.name == binding.name && s.parent_index.is_none())
        {
            binding.binding_kind = "symbol".into();
            binding.symbol_index = Some(si);
        }
    }
    // Resolve bare field reads in languages with implicit member lookup. Existing
    // bindings remain authoritative for locals and parameters (shadowing).
    super::scope::resolve_locals_for(out, language.group());
}

fn visit(n: Node<'_>, language: Language, src: &str, out: &mut ParsedFile) {
    let k = n.kind();
    // Existing analyzers retain their ownership/reference work. Match the exact
    // declaration first, then descendants sharing its name (e.g. variable wrappers).
    let matching: Vec<usize> = out
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.start_byte == n.start_byte() as i64 && s.end_byte == n.end_byte() as i64)
        .map(|(i, _)| i)
        .collect();
    for i in matching {
        fill(&mut out.symbols[i], n, language, src);
    }

    let extra = match k {
        "constructor_declaration" | "compact_constructor_declaration" | "secondary_constructor" => {
            Some("constructor")
        }
        "union_item" => Some("union"),
        "const_item" | "const_spec" => Some("constant"),
        "static_item" | "var_spec" => Some("variable"),
        "enum_variant" | "enum_constant" | "enum_entry" => Some("enum_member"),
        "type_alias" => Some("type_alias"),
        "internal_module" => Some("namespace"),
        "method_signature" | "function_signature" | "method_elem" | "method_spec" => Some("method"),
        "field_declaration" if language == Language::Rust || language == Language::Go => {
            Some("field")
        }
        "public_field_definition" | "property_signature" => Some("field"),
        "property_declaration" if language == Language::Kotlin && !inside_callable(n) => {
            Some("property")
        }
        _ => None,
    };
    if let Some(kind) = extra
        && !(language == Language::Go
            && matches!(k, "var_spec" | "const_spec")
            && inside_callable(n))
        && let Some(nn) = name(n)
    {
        add(out, n, nn, kind, language, src);
    }
    if language == Language::Go && matches!(k, "var_spec" | "const_spec") && !inside_callable(n) {
        for nn in n.children_by_field_name("name", &mut n.walk()) {
            add(
                out,
                n,
                nn,
                if k == "const_spec" {
                    "constant"
                } else {
                    "variable"
                },
                language,
                src,
            );
        }
    }
    if matches!(k, "primary_constructor" | "secondary_constructor")
        && language == Language::Kotlin
        && let Some(nn) = ancestor_name(n)
    {
        add(out, n, nn, "constructor", language, src);
    }
    if k == "companion_object"
        && let Some(nn) = name(n).or_else(|| n.child(0))
    {
        add(out, n, nn, "object", language, src);
    }
    if k == "class_parameter" && language == Language::Kotlin {
        let raw = text(n, src);
        if raw.split_whitespace().any(|w| matches!(w, "val" | "var"))
            && let Some(nn) = name(n)
        {
            add(out, n, nn, "property", language, src);
        }
    }
    if matches!(k, "field_declaration" | "constant_declaration") && language == Language::Java {
        for d in children(n)
            .into_iter()
            .filter(|d| d.kind() == "variable_declarator")
        {
            if let Some(nn) = name(d) {
                add(out, n, nn, "field", language, src);
            }
        }
    }
    if k == "field_declaration" && language == Language::Go {
        for nn in children(n)
            .into_iter()
            .filter(|n| n.kind() == "field_identifier")
        {
            add(out, n, nn, "field", language, src);
        }
        if !children(n).iter().any(|n| n.kind() == "field_identifier")
            && let Some(ty) = n.child_by_field_name("type")
            && let Some(s) = out.symbols.iter_mut().find(|s| {
                s.start_byte == n.start_byte() as i64
                    && s.name == text(ty, src).trim_start_matches('*')
            })
        {
            s.semantic
                .language_details
                .insert("embedded".into(), true.into());
        }
    }
    // Kotlin properties wrap a variable declaration, whose name is not a field.
    if k == "property_declaration"
        && language == Language::Kotlin
        && !inside_callable(n)
        && let Some(vd) = child(n, "variable_declaration")
        && let Some(nn) = name(vd)
    {
        add(out, n, nn, "property", language, src);
    }
    if k == "assignment"
        && language == Language::Python
        && let Some(lhs) = n.child_by_field_name("left")
    {
        let member = lhs.kind() == "attribute"
            && field_text(lhs, "object", src).is_some_and(|s| s == "self" || s == "cls");
        let module_or_class = !inside_callable(n);
        if member {
            if let Some(nn) = lhs.child_by_field_name("attribute") {
                add(out, n, nn, "field", language, src);
            }
        } else if module_or_class && lhs.kind() == "identifier" {
            let kind = if ancestor_name(n).is_some() {
                "field"
            } else {
                "variable"
            };
            add(out, n, lhs, kind, language, src);
        }
    }
    for c in children(n) {
        visit(c, language, src, out);
    }
}
fn inside_callable(mut n: Node<'_>) -> bool {
    while let Some(p) = n.parent() {
        if matches!(
            p.kind(),
            "function_declaration"
                | "function_definition"
                | "method_declaration"
                | "function_item"
                | "method_definition"
                | "lambda_literal"
        ) {
            return true;
        }
        if matches!(
            p.kind(),
            "class_declaration" | "class_definition" | "object_declaration"
        ) {
            return false;
        }
        n = p;
    }
    false
}

fn bind_declarations(root: Node<'_>, language: Language, src: &str, out: &mut ParsedFile) {
    let mut pending = vec![root];
    let mut declarations = Vec::new();
    while let Some(n) = pending.pop() {
        if matches!(n.kind(), "block" | "statement_block")
            && !out
                .scopes
                .iter()
                .any(|s| s.start_byte == n.start_byte() as i64 && s.end_byte == n.end_byte() as i64)
        {
            let owner = out
                .scopes
                .iter()
                .filter(|s| {
                    s.start_byte <= n.start_byte() as i64 && s.end_byte >= n.end_byte() as i64
                })
                .min_by_key(|s| s.end_byte - s.start_byte)
                .and_then(|s| s.owner_symbol_index);
            out.scopes.push(crate::cache::models::NewScope {
                kind: "block".into(),
                start_byte: n.start_byte() as i64,
                end_byte: n.end_byte() as i64,
                owner_symbol_index: owner,
                parent_index: None,
            });
        }
        if matches!(
            n.kind(),
            "variable_declarator"
                | "variable_declaration"
                | "let_declaration"
                | "assignment"
                | "short_var_declaration"
                | "parameter"
                | "formal_parameter"
                | "typed_parameter"
                | "required_parameter"
                | "optional_parameter"
        ) {
            let target = n
                .child_by_field_name("name")
                .or_else(|| n.child_by_field_name("pattern"))
                .or_else(|| n.child_by_field_name("left"))
                .or_else(|| name(n));
            if let Some(nn) = target {
                declarations.push((text(nn, src).to_string(), nn.start_byte() as i64));
            }
        }
        pending.extend(children(n));
    }
    // Give block scopes a real lexical parent and propagate their callable owner.
    for i in 1..out.scopes.len() {
        let s = &out.scopes[i];
        let parent = out
            .scopes
            .iter()
            .enumerate()
            .filter(|(j, p)| {
                *j != i
                    && p.start_byte <= s.start_byte
                    && p.end_byte >= s.end_byte
                    && (p.start_byte < s.start_byte || p.end_byte > s.end_byte)
            })
            .min_by_key(|(j, p)| {
                (
                    p.end_byte - p.start_byte,
                    Some(*j) != s.parent_index,
                    p.owner_symbol_index.is_none(),
                )
            })
            .map(|(j, _)| j);
        if parent.is_some() {
            out.scopes[i].parent_index = parent;
        }
    }
    for scope in &mut out.scopes {
        if scope.kind == "block" {
            scope.owner_symbol_index = out
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.semantic.kind.callable()
                        && s.start_byte <= scope.start_byte
                        && s.end_byte >= scope.end_byte
                })
                .min_by_key(|(_, s)| s.end_byte - s.start_byte)
                .map(|(i, _)| i)
                .or(scope.owner_symbol_index);
        }
    }
    for binding in &mut out.bindings {
        if let Some(si) = binding.symbol_index {
            binding.declaration_byte = out.symbols.get(si).map(|s| s.start_byte);
        } else if let Some(scope) = out.scopes.get(binding.scope_index) {
            binding.declaration_byte = declarations
                .iter()
                .filter(|(name, at)| {
                    *name == binding.name && *at >= scope.start_byte && *at < scope.end_byte
                })
                .map(|(_, at)| *at)
                .min();
        }
        if matches!(binding.binding_kind.as_str(), "local")
            && language != Language::Python
            && let Some(at) = binding.declaration_byte
            && let Some((index, _)) = out
                .scopes
                .iter()
                .enumerate()
                .filter(|(_, s)| s.start_byte <= at && s.end_byte > at)
                .min_by_key(|(_, s)| s.end_byte - s.start_byte)
        {
            binding.scope_index = index;
        }
    }
}

fn member_references(root: Node<'_>, language: Language, src: &str, out: &mut ParsedFile) {
    let mut pending = vec![root];
    while let Some(n) = pending.pop() {
        let explicit = matches!(n.kind(), "field_access" | "member_expression" | "attribute");
        let member = if explicit {
            n.child_by_field_name("field")
                .or_else(|| n.child_by_field_name("property"))
                .or_else(|| n.child_by_field_name("attribute"))
        } else if n.kind() == "identifier"
            && super::semantic::rules(language.group()).implicit_members
        {
            let expression = n.parent().is_some_and(|p| {
                matches!(
                    p.kind(),
                    "return_statement"
                        | "binary_expression"
                        | "assignment_expression"
                        | "argument_list"
                        | "value_argument"
                        | "navigation_expression"
                )
            });
            (expression
                && out.symbols.iter().any(|s| {
                    s.name == text(n, src)
                        && matches!(s.semantic.kind, SymbolKind::Field | SymbolKind::Property)
                }))
            .then_some(n)
        } else {
            None
        };
        if let Some(nn) = member
            && !out
                .refs
                .iter()
                .any(|r| r.name_start_byte.unwrap_or(r.start_byte) == nn.start_byte() as i64)
        {
            let receiver = if explicit {
                field_text(n, "object", src)
            } else {
                None
            };
            let write = n.parent().is_some_and(|p| {
                matches!(p.kind(), "assignment_expression" | "assignment")
                    && p.child_by_field_name("left")
                        .is_some_and(|l| l.id() == n.id())
            });
            out.refs.push(crate::cache::models::NewRef {
                argument_types: None,
                name: text(nn, src).into(),
                ref_kind: if write { "write" } else { "read" }.into(),
                receiver_kind: if receiver.is_some() { "value" } else { "none" }.into(),
                receiver,
                start_line: nn.start_position().row as i64 + 1,
                start_byte: nn.start_byte() as i64,
                name_start_byte: Some(nn.start_byte() as i64),
                ..Default::default()
            });
        }
        pending.extend(children(n));
    }
}

fn argument_types(root: Node<'_>, language: Language, src: &str, out: &mut ParsedFile) {
    let mut pending = vec![root];
    while let Some(n) = pending.pop() {
        let args = n
            .child_by_field_name("arguments")
            .or_else(|| child(n, "value_arguments"));
        if matches!(
            n.kind(),
            "call_expression" | "method_invocation" | "object_creation_expression"
        ) && let Some(args) = args
        {
            let types: Vec<_> = children(args)
                .into_iter()
                .filter(|n| !n.kind().contains("comment"))
                .map(|arg| {
                    let arg = if arg.kind() == "value_argument" {
                        arg.named_child(0).unwrap_or(arg)
                    } else {
                        arg
                    };
                    let raw = text(arg, src);
                    let literal = match arg.kind() {
                        "string" | "string_literal" | "interpreted_string_literal" => Some(
                            if language == Language::Java || language == Language::Kotlin {
                                "String"
                            } else {
                                "string"
                            },
                        ),
                        "decimal_integer_literal"
                        | "hex_integer_literal"
                        | "number_literal"
                        | "integer_literal"
                        | "integer"
                        | "number" => match language {
                            Language::Java => Some(if raw.ends_with(['l', 'L']) {
                                "long"
                            } else if raw.contains('.') {
                                "double"
                            } else {
                                "int"
                            }),
                            Language::Kotlin => Some(if raw.ends_with('L') {
                                "Long"
                            } else if raw.contains('.') {
                                "Double"
                            } else {
                                "Int"
                            }),
                            Language::TypeScript
                            | Language::Tsx
                            | Language::JavaScript
                            | Language::Jsx => Some("number"),
                            _ => None,
                        },
                        "true" | "false" | "boolean_literal" => Some(
                            if matches!(
                                language,
                                Language::Java
                                    | Language::TypeScript
                                    | Language::Tsx
                                    | Language::JavaScript
                                    | Language::Jsx
                            ) {
                                "boolean"
                            } else {
                                "bool"
                            },
                        ),
                        _ => None,
                    };
                    literal.map(str::to_string).or_else(|| {
                        if arg.kind() != "identifier" {
                            return None;
                        }
                        out.bindings
                            .iter()
                            .filter(|b| {
                                b.name == raw
                                    && out.scopes.get(b.scope_index).is_some_and(|s| {
                                        s.start_byte <= arg.start_byte() as i64
                                            && s.end_byte > arg.end_byte() as i64
                                    })
                            })
                            .min_by_key(|b| {
                                out.scopes[b.scope_index].end_byte
                                    - out.scopes[b.scope_index].start_byte
                            })
                            .and_then(|b| b.type_expr.clone())
                    })
                })
                .collect();
            for reference in &mut out.refs {
                let at = reference.name_start_byte.unwrap_or(reference.start_byte);
                if reference.ref_kind == "call"
                    && at >= n.start_byte() as i64
                    && at < args.start_byte() as i64
                {
                    reference.arg_count = Some(types.len() as i64);
                    reference.argument_types = Some(types.clone());
                }
            }
        }
        pending.extend(children(n));
    }
}
fn add(out: &mut ParsedFile, n: Node<'_>, nn: Node<'_>, kind: &str, language: Language, src: &str) {
    let nm = text(nn, src);
    if nm.is_empty() {
        return;
    }
    if let Some(s) = out.symbols.iter_mut().find(|s| {
        s.name == nm && s.start_byte >= n.start_byte() as i64 && s.end_byte <= n.end_byte() as i64
    }) {
        fill(s, n, language, src);
        s.semantic.kind = SymbolKind::from_legacy(kind);
        s.semantic.name_range = Some([nn.start_byte(), nn.end_byte()]);
        return;
    }
    let mut s = NewSymbol {
        name: nm.to_string(),
        kind: kind.into(),
        start_byte: n.start_byte() as i64,
        end_byte: n.end_byte() as i64,
        start_line: n.start_position().row as i64 + 1,
        end_line: n.end_position().row as i64 + 1,
        signature: super::node_signature(n, src.as_bytes()),
        ..Default::default()
    };
    fill(&mut s, n, language, src);
    s.semantic.kind = SymbolKind::from_legacy(kind);
    s.semantic.name_range = Some([nn.start_byte(), nn.end_byte()]);
    out.symbols.push(s);
}

fn fill(s: &mut NewSymbol, n: Node<'_>, language: Language, src: &str) {
    let mut facts = SymbolFacts {
        kind: SymbolKind::from_legacy(&s.kind),
        ..Default::default()
    };
    if let Some(nn) = name(n) {
        facts.name_range = Some([nn.start_byte(), nn.end_byte()]);
    }
    let k = n.kind();
    facts.kind = match k {
        "record_declaration" => SymbolKind::Record,
        "object_declaration" | "companion_object" => SymbolKind::Object,
        "constructor_declaration" | "compact_constructor_declaration" | "secondary_constructor" => {
            SymbolKind::Constructor
        }
        "union_item" => SymbolKind::Union,
        "const_item" | "const_spec" => SymbolKind::Constant,
        "type_item" | "type_alias" | "type_alias_declaration" => SymbolKind::TypeAlias,
        "method_definition" if s.name == "constructor" => SymbolKind::Constructor,
        _ => facts.kind,
    };
    if language == Language::Python && s.name == "__init__" {
        facts.kind = SymbolKind::Constructor;
    }
    let header_end = n
        .child_by_field_name("body")
        .or_else(|| {
            children(n).into_iter().find(|c| {
                matches!(
                    c.kind(),
                    "function_body" | "class_body" | "block" | "statement_block"
                )
            })
        })
        .map(|b| b.start_byte())
        .unwrap_or(n.end_byte());
    let prefix_end = name(n).map(|n| n.start_byte()).unwrap_or(header_end);
    let prefix = src.get(n.start_byte()..prefix_end).unwrap_or("");
    let words: Vec<_> = prefix
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    let has = |word: &str| words.contains(&word);
    if language == Language::Kotlin && k == "class_declaration" && has("enum") {
        facts.kind = SymbolKind::Enum;
    }
    if language == Language::Kotlin {
        for modifier in [
            "data", "sealed", "inline", "suspend", "override", "expect", "actual",
        ] {
            if has(modifier) {
                facts.language_details.insert(modifier.into(), true.into());
            }
        }
    }
    facts.visibility = if has("private") {
        Visibility::Private
    } else if has("protected") {
        Visibility::Protected
    } else if has("internal") {
        Visibility::Internal
    } else if has("public") || has("pub") {
        Visibility::Public
    } else {
        match language {
            Language::Java => Visibility::Package,
            Language::Kotlin => Visibility::Public,
            Language::Go => {
                if s.name.chars().next().is_some_and(char::is_uppercase) {
                    Visibility::Public
                } else {
                    Visibility::Package
                }
            }
            Language::Rust => Visibility::Private,
            _ => Visibility::Unknown,
        }
    };
    if prefix.contains("pub(") {
        facts.visibility = Visibility::Restricted;
    }
    facts.modifiers.is_static = has("static").then_some(true);
    facts.modifiers.is_async = has("async").then_some(true);
    facts.modifiers.is_abstract = has("abstract").then_some(true);
    facts.modifiers.is_virtual = has("virtual").then_some(true);
    if language == Language::Kotlin && k == "property_declaration" {
        facts.modifiers.is_mutable = Some(has("var"));
    }
    if matches!(
        language,
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx
    ) && k == "variable_declarator"
        && let Some(parent) = n.parent()
    {
        let declaration = text(parent, src).trim_start();
        if declaration.starts_with("const ") {
            facts.modifiers.is_mutable = Some(false);
        } else if declaration.starts_with("let ") || declaration.starts_with("var ") {
            facts.modifiers.is_mutable = Some(true);
        }
    }
    if k == "method_definition"
        && children(n)
            .iter()
            .any(|n| matches!(n.kind(), "get" | "set"))
    {
        facts.kind = SymbolKind::Property;
        facts.role = SymbolRole::Accessor;
    }
    if language == Language::Rust && matches!(k, "static_item" | "const_item") {
        facts.modifiers.is_mutable = Some(has("mut"));
    }
    facts.declared_type = field_text(n, "type", src).or_else(|| {
        child(n, "type_annotation").map(|t| text(t, src).trim_start_matches(':').trim().to_string())
    });
    facts.return_type = field_text(n, "return_type", src).or_else(|| field_text(n, "result", src));
    if language == Language::Java && facts.kind.callable() {
        facts.return_type = facts.declared_type.take();
    }
    facts.type_parameters = n
        .child_by_field_name("type_parameters")
        .map(children)
        .unwrap_or_default()
        .into_iter()
        .map(|p| text(p, src).to_string())
        .collect();
    let callable_node = if k == "variable_declarator" && facts.kind.callable() {
        n.child_by_field_name("value").unwrap_or(n)
    } else {
        n
    };
    let params = callable_node
        .child_by_field_name("parameters")
        .or_else(|| child(n, "function_value_parameters"))
        .or_else(|| child(n, "class_parameters"));
    if let Some(params) = params {
        facts.parameters = Some(parameters(params, language, src));
        if facts.kind.callable() {
            s.param_count = Some(
                facts
                    .parameters
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|p| !p.receiver)
                    .count() as i64,
            );
        }
    }
    facts.role = if n.child_by_field_name("body").is_some()
        || children(n)
            .iter()
            .any(|c| matches!(c.kind(), "function_body" | "block" | "class_body"))
    {
        SymbolRole::Definition
    } else if facts.kind.callable() {
        SymbolRole::Declaration
    } else {
        SymbolRole::Definition
    };
    if language == Language::Rust && s.kind == "method" {
        let receiver = facts
            .parameters
            .as_ref()
            .is_some_and(|ps| ps.iter().any(|p| p.receiver));
        facts.modifiers.is_static = Some(!receiver);
        if !receiver {
            facts.role = SymbolRole::AssociatedFunction;
        }
    }
    if language == Language::Java && facts.kind.callable() {
        facts.modifiers.is_static = Some(has("static"));
    }
    if language == Language::Kotlin {
        let parts = children(n);
        let nn = name(n);
        let ps = child(n, "function_value_parameters");
        if let (Some(nn), Some(ps)) = (nn, ps) {
            facts.extension_receiver = parts
                .iter()
                .find(|t| {
                    matches!(t.kind(), "user_type" | "nullable_type" | "function_type")
                        && t.end_byte() <= nn.start_byte()
                })
                .map(|t| text(*t, src).to_string());
            facts.return_type = parts
                .iter()
                .find(|t| {
                    matches!(t.kind(), "user_type" | "nullable_type" | "function_type")
                        && t.start_byte() >= ps.end_byte()
                })
                .map(|t| text(*t, src).to_string());
        }
        if k == "property_declaration"
            && let Some(vd) = child(n, "variable_declaration")
        {
            facts.declared_type = children(vd)
                .into_iter()
                .find(|t| t.kind() != "identifier")
                .map(|t| text(t, src).to_string());
        }
    }
    if language == Language::Python {
        let decorated = n.parent().filter(|p| p.kind() == "decorated_definition");
        if let Some(d) = decorated {
            let decorators: Vec<_> = children(d)
                .into_iter()
                .filter(|c| c.kind() == "decorator")
                .map(|c| text(c, src).trim().trim_start_matches('@').to_string())
                .collect();
            if decorators.iter().any(|d| d == "staticmethod") {
                facts.modifiers.is_static = Some(true);
            }
            if decorators.iter().any(|d| d == "classmethod") {
                facts
                    .language_details
                    .insert("classmethod".into(), true.into());
            }
            if decorators.iter().any(|d| d == "property") {
                facts.kind = SymbolKind::Property;
                facts.role = SymbolRole::Accessor;
            }
            facts
                .language_details
                .insert("decorators".into(), serde_json::json!(decorators));
        }
        if s.kind == "method"
            && facts.modifiers.is_static != Some(true)
            && let Some(params) = &mut facts.parameters
            && let Some(first) = params.first_mut()
        {
            first.receiver = true;
        }
    }
    for field in [
        "superclass",
        "interfaces",
        "super_interfaces",
        "super_types",
        "bases",
        "trait",
    ] {
        if let Some(base) = n.child_by_field_name(field) {
            collect_bases(
                base,
                if matches!(field, "interfaces" | "super_interfaces" | "trait") {
                    RelationKind::Implements
                } else {
                    RelationKind::Inherits
                },
                src,
                &mut facts.relations,
            );
        }
    }
    for c in children(n) {
        if matches!(
            c.kind(),
            "delegation_specifiers" | "superclass" | "super_interfaces" | "class_heritage"
        ) {
            collect_bases(c, RelationKind::Inherits, src, &mut facts.relations);
        }
    }
    facts.relations.sort_by(|a, b| a.target.cmp(&b.target));
    facts.relations.dedup_by(|a, b| a.target == b.target);
    if matches!(
        language,
        Language::Rust | Language::Go | Language::Java | Language::Kotlin
    ) {
        s.is_exported = facts.visibility == Visibility::Public;
    }
    s.semantic = facts;
}
fn collect_bases(n: Node<'_>, kind: RelationKind, src: &str, out: &mut Vec<TypeRelation>) {
    if matches!(
        n.kind(),
        "type_identifier" | "user_type" | "scoped_type_identifier" | "generic_type" | "identifier"
    ) {
        out.push(TypeRelation {
            kind,
            target: text(n, src).to_string(),
        });
    } else {
        for c in children(n) {
            collect_bases(c, kind, src, out);
        }
    }
}
fn parameters(n: Node<'_>, language: Language, src: &str) -> Vec<Parameter> {
    let mut out = Vec::new();
    for p in children(n) {
        if language == Language::Kotlin && !matches!(p.kind(), "parameter" | "class_parameter") {
            continue;
        }
        if matches!(
            p.kind(),
            "comment"
                | "line_comment"
                | "block_comment"
                | "positional_separator"
                | "keyword_separator"
        ) {
            continue;
        }
        let raw = text(p, src);
        let mut param = Parameter {
            name: name(p)
                .map(|n| text(n, src).to_string())
                .or_else(|| (p.kind() == "identifier").then(|| raw.to_string())),
            declared_type: field_text(p, "type", src).or_else(|| {
                if language == Language::Kotlin {
                    children(p)
                        .into_iter()
                        .find(|n| {
                            matches!(n.kind(), "user_type" | "nullable_type" | "function_type")
                        })
                        .map(|t| text(t, src).to_string())
                } else {
                    None
                }
            }),
            optional: p.child_by_field_name("value").is_some()
                || p.child_by_field_name("default_value").is_some()
                || matches!(
                    p.kind(),
                    "optional_parameter" | "default_parameter" | "typed_default_parameter"
                ),
            variadic: matches!(
                p.kind(),
                "spread_parameter"
                    | "variadic_parameter_declaration"
                    | "variadic_parameter"
                    | "list_splat_pattern"
                    | "dictionary_splat_pattern"
            ) || raw.trim_start().starts_with('*'),
            receiver: p.kind() == "self_parameter",
        };
        if language == Language::Kotlin {
            param.variadic |= raw.split_whitespace().any(|w| w == "vararg")
                || p.prev_named_sibling()
                    .is_some_and(|n| text(n, src).split_whitespace().any(|w| w == "vararg"));
            param.optional |=
                p.next_sibling().is_some_and(|n| n.kind() == "=") || raw.contains('=');
        }
        if language == Language::TypeScript || language == Language::Tsx {
            param.variadic |= p
                .child_by_field_name("pattern")
                .or_else(|| p.child_by_field_name("name"))
                .is_some_and(|n| n.kind() == "rest_pattern");
        }
        if language == Language::Rust && raw.contains("self") && p.kind() == "self_parameter" {
            param.receiver = true;
        }
        if let Some(t) = &mut param.declared_type {
            *t = t.trim_start_matches(':').trim().to_string();
        }
        let names: Vec<_> = if language == Language::Go {
            p.children_by_field_name("name", &mut p.walk()).collect()
        } else {
            Vec::new()
        };
        if names.len() > 1 {
            for nn in names {
                let mut copy = param.clone();
                copy.name = Some(text(nn, src).to_string());
                out.push(copy);
            }
        } else {
            out.push(param);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(language: Language, src: &str) -> ParsedFile {
        super::super::parse_file(language, src)
    }
    #[test]
    fn java_constructor_fields_packages_overloads_and_unicode() {
        let source = "package one.deep; class Café { private int value; Café(int x) {} int get(int x) { return x; } int get(String x) { return value; } }";
        let out = parse(Language::Java, source);
        assert!(out.parse_ok);
        let ctor = out
            .symbols
            .iter()
            .find(|s| s.semantic.kind == SymbolKind::Constructor)
            .unwrap();
        assert_eq!(ctor.semantic.qualified_name, "one::deep::Café::Café");
        let field = out
            .symbols
            .iter()
            .find(|s| s.semantic.kind == SymbolKind::Field)
            .unwrap();
        assert_eq!(field.semantic.visibility, Visibility::Private);
        assert_eq!(field.semantic.declared_type.as_deref(), Some("int"));
        let range = field.semantic.name_range.unwrap();
        assert_eq!(&source[range[0]..range[1]], "value");
        let overloads: Vec<_> = out.symbols.iter().filter(|s| s.name == "get").collect();
        assert_eq!(overloads.len(), 2);
        assert_ne!(
            overloads[0].semantic.parameters.as_ref().unwrap()[0].declared_type,
            overloads[1].semantic.parameters.as_ref().unwrap()[0].declared_type
        );
    }
    #[test]
    fn rust_preserves_union_constants_and_associated_functions() {
        let out = parse(
            Language::Rust,
            "union Bits { x: u32, y: f32 } const N: usize = 1; struct A; impl A { fn new() -> A { A } fn run(&self) {} }",
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::Union)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::Constant)
        );
        assert_eq!(
            out.symbols
                .iter()
                .find(|s| s.name == "new")
                .unwrap()
                .semantic
                .role,
            SymbolRole::AssociatedFunction
        );
        assert_eq!(
            out.symbols
                .iter()
                .find(|s| s.name == "run")
                .unwrap()
                .semantic
                .modifiers
                .is_static,
            Some(false)
        );
    }
    #[test]
    fn python_known_and_unknown_decorators() {
        let out = parse(
            Language::Python,
            "class C:\n @staticmethod\n def make(x: int = 1):\n  return x\n @property\n def value(self):\n  return 1\n @magic\n def other(self):\n  pass\n",
        );
        let make = out.symbols.iter().find(|s| s.name == "make").unwrap();
        assert_eq!(make.semantic.modifiers.is_static, Some(true));
        assert!(make.semantic.accepts_arity(0));
        assert!(
            out.symbols
                .iter()
                .any(|s| s.name == "value" && s.semantic.kind == SymbolKind::Property)
        );
        assert_eq!(
            out.symbols
                .iter()
                .find(|s| s.name == "other")
                .unwrap()
                .semantic
                .modifiers
                .is_static,
            None
        );
    }
    #[test]
    fn go_grouped_parameters_and_interface_owner() {
        let out = parse(
            Language::Go,
            "package p\ntype I interface { Run(x int) }\nfunc Sum(a, b int) int { return a+b }\n",
        );
        let sum = out.symbols.iter().find(|s| s.name == "Sum").unwrap();
        assert_eq!(sum.semantic.parameters.as_ref().unwrap().len(), 2);
        assert_eq!(sum.semantic.qualified_name, "p::Sum");
        assert_eq!(
            out.symbols
                .iter()
                .find(|s| s.name == "Run")
                .unwrap()
                .semantic
                .owner
                .as_deref(),
            Some("p::I")
        );
    }
    #[test]
    fn typescript_constructor_optional_parameter_and_alias() {
        let out = parse(
            Language::TypeScript,
            "type ID = string; class A { value: number; constructor(x: number) {} run(x?: string) {} }",
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::TypeAlias)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::Constructor)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::Field)
        );
        assert!(
            out.symbols
                .iter()
                .find(|s| s.name == "run")
                .unwrap()
                .semantic
                .accepts_arity(0)
        );
    }
    #[test]
    fn kotlin_objects_properties_and_functions() {
        let out = parse(
            Language::Kotlin,
            "package p\nobject Store { val count: Int = 1\n fun run(x: Int = 1) {} }\nfun top() {}\n",
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.semantic.kind == SymbolKind::Object)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.name == "count" && s.semantic.kind == SymbolKind::Property)
        );
        assert_eq!(
            out.symbols
                .iter()
                .find(|s| s.name == "top")
                .unwrap()
                .semantic
                .kind,
            SymbolKind::Function
        );
    }
    #[test]
    fn sfc_keeps_component_and_script_source_ranges() {
        for language in [Language::Vue, Language::Svelte] {
            let source = "<script lang=\"ts\">\nfunction hello(x: string) { return x; }\n</script>\n<p>hi</p>";
            let out = parse(language, source);
            let hello = out.symbols.iter().find(|s| s.name == "hello").unwrap();
            let range = hello.semantic.name_range.unwrap();
            assert_eq!(&source[range[0]..range[1]], "hello");
            assert!(
                out.symbols
                    .iter()
                    .any(|s| s.semantic.kind == SymbolKind::Component)
            );
        }
    }
}
