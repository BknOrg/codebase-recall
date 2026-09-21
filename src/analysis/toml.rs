//! Configuration keys, indexed so a setting read in code can be traced to the
//! file that defines it.
//!
//! `settings.get_string("revsets.run")` already lands in `string_literals` as
//! the literal `revsets.run`. Recording `[revsets] run = "@"` under the same
//! spelling is what lets one `search` show both the read and the default.
//!
//! This is deliberately a line scanner, not a TOML parser: it needs the key
//! paths and their lines, not the values, and it must not fail on a file it
//! only half understands.

use crate::analysis::ParsedFile;
use crate::cache::models::NewStringLiteral;

/// Marker stored in `string_literals.callee` for a key *defined* in config,
/// which distinguishes it from the same key being *read* in code.
const DEFINITION: &str = "<toml-key>";

pub fn parse(source: &str) -> ParsedFile {
    let mut out = ParsedFile {
        // A config file has no symbols to extract, so the walk always
        // "succeeds"; marking it otherwise would report a parse failure the
        // user cannot act on.
        parse_ok: true,
        ..ParsedFile::default()
    };

    let mut table = String::new();
    for (i, raw) in source.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = table_header(line) {
            table = header;
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let key = unquote(key.trim());
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue; // a continuation line of a multi-line value, not a key
        }
        let value = if table.is_empty() {
            key
        } else {
            format!("{table}.{key}")
        };
        out.string_literals.push(NewStringLiteral {
            value,
            callee: Some(DEFINITION.to_string()),
            line: Some(i as i64 + 1),
        });
    }

    out
}

/// `[a.b]` and `[[a.b]]` both scope the keys that follow to `a.b`.
fn table_header(line: &str) -> Option<String> {
    let inner = line
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .or_else(|| line.strip_prefix('[').and_then(|s| s.strip_suffix(']')))?;
    Some(
        inner
            .split('.')
            .map(|seg| unquote(seg.trim()))
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Drop a trailing `#` comment, but not one inside a quoted value.
fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let mut quote = '"';
    for (i, c) in line.char_indices() {
        match c {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote = c;
            }
            c if in_quotes && c == quote => in_quotes = false,
            '#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn keys_are_indexed_under_their_table() {
        let src = r#"
# a comment
edition = "2024"

[revsets]
run = "@"          # trailing comment
"log" = "x"

[[bin]]
name = "code-rcl"

[tool.ruff.lint]
select = ["E"]
"#;
        let p = parse(src);
        assert!(p.parse_ok);
        let keys: Vec<&str> = p.string_literals.iter().map(|s| s.value.as_str()).collect();

        assert!(keys.contains(&"edition"), "root key: {keys:?}");
        assert!(keys.contains(&"revsets.run"), "table key: {keys:?}");
        assert!(keys.contains(&"revsets.log"), "quoted key: {keys:?}");
        assert!(keys.contains(&"bin.name"), "array-of-tables key: {keys:?}");
        assert!(keys.contains(&"tool.ruff.lint.select"), "nested: {keys:?}");

        // A config file defines no symbols; only the keys are recorded.
        assert!(p.symbols.is_empty());
        assert!(p.refs.is_empty());
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let p = parse("[colors]\naccent = \"#ff0000\"\n");
        let keys: Vec<&str> = p.string_literals.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(keys, vec!["colors.accent"]);
    }

    #[test]
    fn the_line_number_points_at_the_definition() {
        let p = parse("[a]\nb = 1\n");
        assert_eq!(p.string_literals[0].line, Some(2));
    }
}
