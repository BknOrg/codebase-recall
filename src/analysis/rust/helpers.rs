use tree_sitter::Node;

/// Strip references, lifetimes and generics down to the leading path's last
/// segment: `&mut Vec<Foo>` -> `Vec`, `crate::a::Bar` -> `Bar`.
pub fn simple_type_name(raw: &str) -> String {
    let t = raw.trim().trim_start_matches('&').trim();
    let t = t.strip_prefix("mut ").unwrap_or(t).trim();
    let head = t.split(['<', ' ', '(']).next().unwrap_or(t);
    head.rsplit("::").next().unwrap_or(head).to_string()
}

pub fn clean_rust_string_literal(raw: &str) -> String {
    let s = raw.trim();
    if s.starts_with("r#\"") && s.ends_with("\"#") && s.len() >= 5 {
        return s[3..s.len() - 2].to_string();
    }
    if s.starts_with("r##\"") && s.ends_with("\"##") && s.len() >= 7 {
        return s[4..s.len() - 3].to_string();
    }
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// `foo::bar` / `Foo::bar` -> path; `expr.method()` -> value (or self); bare -> none.
pub fn classify_rust_receiver(func_kind: &str, receiver: Option<&str>) -> &'static str {
    match (func_kind, receiver) {
        (_, None) => "none",
        ("scoped_identifier", _) => "path",
        (_, Some("self")) | (_, Some("Self")) => "self",
        _ => "value",
    }
}

pub fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

pub fn join(prefix: &str, seg: &str) -> String {
    match (prefix.is_empty(), seg.is_empty()) {
        (true, _) => seg.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}::{seg}"),
    }
}
