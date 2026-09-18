pub fn extract_signature(
    lines: &[String],
    start_line: Option<i64>,
    end_line: Option<i64>,
) -> String {
    let Some(start) = start_line else {
        return String::new();
    };
    let start_idx = (start.max(1) - 1) as usize;
    if start_idx >= lines.len() {
        return String::new();
    }

    let end_idx = match end_line {
        Some(end) => (end.max(start) - 1) as usize,
        None => start_idx,
    };

    let mut sig = String::new();
    let max_lines = (end_idx - start_idx + 1).min(5);

    for i in 0..max_lines {
        let idx = start_idx + i;
        if idx >= lines.len() {
            break;
        }
        let line = lines[idx].trim();
        if line.starts_with("//") || line.starts_with('#') || line.starts_with("/*") {
            continue;
        }
        if !sig.is_empty() {
            sig.push(' ');
        }
        if let Some(pos) = line.find('{') {
            sig.push_str(line[..pos].trim_end());
            break;
        } else if let Some(pos) = line.find(';') {
            sig.push_str(line[..=pos].trim_end());
            break;
        } else if let Some(pos) = line.find('=') {
            let before = line[..pos].trim_end();
            if before.contains("fn ") || before.contains("def ") || before.contains("let ") {
                sig.push_str(before);
                break;
            } else {
                sig.push_str(line);
            }
        } else {
            sig.push_str(line);
        }
    }

    sig.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn extract_docs_and_diagram(
    lines: &[String],
    start_line: Option<i64>,
    end_line: Option<i64>,
    language: &str,
    max_doc_lines: usize,
) -> (Option<String>, bool) {
    let Some(start) = start_line else {
        return (None, false);
    };
    let start_idx = (start.max(1) - 1) as usize;
    if start_idx >= lines.len() {
        return (None, false);
    }

    let mut raw_lines = Vec::new();

    if language.eq_ignore_ascii_case("python") {
        let end_idx = match end_line {
            Some(end) => (end.max(start) - 1) as usize,
            None => start_idx,
        };
        let mut in_docstring = false;
        let mut quote_marker = "";
        let check_limit = (start_idx + 15).min(end_idx + 1).min(lines.len());
        for i in start_idx..check_limit {
            let line = lines[i].trim();
            if !in_docstring {
                if line.starts_with("\"\"\"") {
                    quote_marker = "\"\"\"";
                    in_docstring = true;
                    raw_lines.push(line);
                    if line.len() > 3 && line[3..].contains("\"\"\"") {
                        break;
                    }
                } else if line.starts_with("'''") {
                    quote_marker = "'''";
                    in_docstring = true;
                    raw_lines.push(line);
                    if line.len() > 3 && line[3..].contains("'''") {
                        break;
                    }
                }
            } else {
                raw_lines.push(line);
                if line.contains(quote_marker) {
                    break;
                }
            }
        }
    } else {
        let mut cur = start_idx.saturating_sub(1);
        let mut collected_rev = Vec::new();
        while cur < lines.len() && start_idx > 0 {
            let trimmed = lines[cur].trim();
            if trimmed.is_empty() {
                if !collected_rev.is_empty() {
                    break;
                }
                if cur == 0 {
                    break;
                }
                cur -= 1;
                continue;
            }
            if trimmed.starts_with("#[") || trimmed.starts_with('@') || trimmed == "]" {
                if cur == 0 {
                    break;
                }
                cur -= 1;
                continue;
            }
            if trimmed.starts_with("///")
                || trimmed.starts_with("//!")
                || trimmed.starts_with("/**")
                || trimmed.starts_with('*')
                || trimmed.starts_with("//")
            {
                collected_rev.push(trimmed);
                if cur == 0 {
                    break;
                }
                cur -= 1;
                continue;
            }
            break;
        }
        collected_rev.reverse();

        if collected_rev.is_empty() {
            let end_check = (start_idx + 5).min(lines.len());
            for i in start_idx..end_check {
                let trimmed = lines[i].trim();
                if trimmed.starts_with("///") || trimmed.starts_with("/**") {
                    collected_rev.push(trimmed);
                } else if !collected_rev.is_empty() {
                    break;
                }
            }
        }
        raw_lines = collected_rev;
    }

    let has_diagram = raw_lines.iter().any(|l| is_diagram_fence_opening(l));

    if max_doc_lines == 0 {
        return (None, has_diagram);
    }

    let mut narrative_lines = Vec::new();
    let mut in_fence = false;

    for line in raw_lines {
        if is_fence_delimiter(&line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        let cleaned = strip_doc_prefix(&line);
        if !cleaned.is_empty() {
            narrative_lines.push(cleaned.to_string());
            if narrative_lines.len() >= max_doc_lines {
                break;
            }
        }
    }

    let doc = if narrative_lines.is_empty() {
        None
    } else {
        Some(narrative_lines.join("\n"))
    };

    (doc, has_diagram)
}

pub fn strip_doc_prefix(line: &str) -> &str {
    line.trim()
        .trim_start_matches("///")
        .trim_start_matches("//!")
        .trim_start_matches("/**")
        .trim_end_matches("*/")
        .trim_start_matches('*')
        .trim_start_matches("//")
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("'''")
        .trim()
}

pub fn is_fence_delimiter(line: &str) -> bool {
    let trimmed = strip_doc_prefix(line);
    let is_backtick = trimmed.starts_with("```");
    let is_tilde = trimmed.starts_with("~~~");
    if is_backtick || is_tilde {
        let fence_char = if is_backtick { '`' } else { '~' };
        let count = trimmed.chars().take_while(|&c| c == fence_char).count();
        if count >= 3 {
            let rest = trimmed[count..].trim();
            return !rest.contains(fence_char);
        }
    }
    false
}

pub fn is_diagram_fence_opening(line: &str) -> bool {
    let trimmed = strip_doc_prefix(line);
    let is_backtick = trimmed.starts_with("```");
    let is_tilde = trimmed.starts_with("~~~");
    if is_backtick || is_tilde {
        let fence_char = if is_backtick { '`' } else { '~' };
        let count = trimmed.chars().take_while(|&c| c == fence_char).count();
        if count >= 3 {
            let rest = trimmed[count..].trim();
            if rest.is_empty() || rest.contains(fence_char) {
                return false;
            }
            let lang = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();

            return lang == "text" || lang == "diagram" || lang == "mermaid";
        }
    }
    false
}

pub fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("//")
        || trimmed.starts_with("/**")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with('#')
        || trimmed.starts_with("\"\"\"")
        || trimmed.starts_with("'''")
}

pub fn check_body_has_diagram(
    lines: &[String],
    start_line: Option<i64>,
    end_line: Option<i64>,
) -> bool {
    let (Some(start), Some(end)) = (start_line, end_line) else {
        return false;
    };
    let start_idx = (start.max(1) - 1) as usize;
    let end_idx = (end.max(start) - 1) as usize;
    if start_idx >= lines.len() {
        return false;
    }
    let limit = (end_idx + 1).min(lines.len());
    for line in &lines[start_idx..limit] {
        if is_comment_line(line) && is_diagram_fence_opening(line) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::digest::models::{SymbolItem, TypeDigest};

    #[test]
    fn test_extract_docs_and_diagram_strips_fences_and_flags_diagram() {
        let lines: Vec<String> = vec![
            "/// First line of doc".to_string(),
            "/// Second line of doc".to_string(),
            "/// ```text".to_string(),
            "/// [A] -> [B]".to_string(),
            "/// ```".to_string(),
            "/// Post diagram note".to_string(),
            "pub fn test_target() {}".to_string(),
        ];

        let (doc, has_diag) = extract_docs_and_diagram(&lines, Some(7), Some(7), "rust", 3);
        assert!(has_diag, "Should detect fenced code block diagram");
        assert!(doc.is_some());
        let d = doc.unwrap();
        assert!(d.contains("First line of doc"));
        assert!(d.contains("Second line of doc"));
        assert!(
            !d.contains("[A] -> [B]"),
            "Fenced code block content must be stripped from doc preview"
        );
    }

    #[test]
    fn test_extract_docs_limit_zero() {
        let lines: Vec<String> = vec![
            "/// First line of doc".to_string(),
            "pub fn test_target() {}".to_string(),
        ];

        let (doc, has_diag) = extract_docs_and_diagram(&lines, Some(2), Some(2), "rust", 0);
        assert_eq!(doc, None);
        assert!(!has_diag);
    }

    #[test]
    fn test_has_diagram_ignores_shell_and_rust_fences() {
        let lines: Vec<String> = vec![
            "/// Example usage:".to_string(),
            "/// ```sh".to_string(),
            "/// jj interdiff --from x --to y".to_string(),
            "/// ```".to_string(),
            "/// Also shell:".to_string(),
            "/// ```shell".to_string(),
            "/// cargo run".to_string(),
            "/// ```".to_string(),
            "/// Also rust:".to_string(),
            "/// ```rust".to_string(),
            "/// let x = 1;".to_string(),
            "/// ```".to_string(),
            "pub struct RunArgs {}".to_string(),
        ];

        let (doc, has_diag) = extract_docs_and_diagram(&lines, Some(13), Some(13), "rust", 5);
        assert!(
            !has_diag,
            "Should NOT flag shell or rust code fences as diagrams"
        );
        assert!(doc.is_some());
        let d = doc.unwrap();
        assert!(d.contains("Example usage:"));
        assert!(!d.contains("jj interdiff"));
        assert!(!d.contains("cargo run"));
    }

    #[test]
    fn test_has_diagram_json_explicit_false() {
        let sym = SymbolItem {
            name: "MetaeditArgs".to_string(),
            kind: "struct".to_string(),
            signature: "pub struct MetaeditArgs".to_string(),
            line: Some(50),
            is_exported: true,
            doc: None,
            has_diagram: false,
        };

        let json_str = serde_json::to_string(&sym).unwrap();
        assert!(
            json_str.contains("\"has_diagram\":false"),
            "has_diagram: false must be explicitly serialized in JSON, got: {}",
            json_str
        );
    }

    #[test]
    fn test_has_diagram_in_fields_detected() {
        let lines: Vec<String> = vec![
            "/// Top-level struct doc without diagram".to_string(),
            "pub struct NewArgs {".to_string(),
            "    /// Insert after commit".to_string(),
            "    /// ```text".to_string(),
            "    ///   A -> B".to_string(),
            "    /// ```".to_string(),
            "    pub insert_after: Option<String>,".to_string(),
            "}".to_string(),
        ];

        let has_diag_fields = check_body_has_diagram(&lines, Some(2), Some(8));
        assert!(
            has_diag_fields,
            "Should detect ```text diagram inside struct body/fields"
        );

        let ty = TypeDigest {
            name: "NewArgs".to_string(),
            kind: "struct".to_string(),
            signature: "pub struct NewArgs".to_string(),
            doc: Some("Top-level struct doc without diagram".to_string()),
            has_diagram: has_diag_fields,
            has_diagram_in_fields: has_diag_fields,
            methods: Vec::new(),
        };

        let json_str = serde_json::to_string(&ty).unwrap();
        assert!(json_str.contains("\"has_diagram\":true"));
        assert!(json_str.contains("\"has_diagram_in_fields\":true"));
    }

    #[test]
    fn test_has_diagram_in_fields_ignores_shell() {
        let lines: Vec<String> = vec![
            "/// Struct doc".to_string(),
            "pub struct MetaeditArgs {".to_string(),
            "    /// Example shell:".to_string(),
            "    /// ```shell".to_string(),
            "    /// jj metaedit".to_string(),
            "    /// ```".to_string(),
            "    pub author: Option<String>,".to_string(),
            "}".to_string(),
        ];

        let has_diag_fields = check_body_has_diagram(&lines, Some(2), Some(8));
        assert!(
            !has_diag_fields,
            "Should NOT flag ```shell in fields as diagram"
        );
    }
}
