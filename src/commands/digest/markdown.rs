use crate::commands::digest::models::DigestReport;

pub fn format_markdown_digest(report: &DigestReport) -> String {
    let mut md = String::new();

    md.push_str(&format!("# Architecture Digest: {}\n\n", report.project));

    let mut langs: Vec<_> = report.languages.iter().collect();
    langs.sort_by(|a, b| b.1.cmp(a.1));
    let lang_summary = langs
        .iter()
        .map(|(l, count)| format!("{l} ({count})"))
        .collect::<Vec<_>>()
        .join(", ");

    md.push_str(&format!(
        "**Summary**: {} files | {} symbols ({} public) | Languages: {}\n\n",
        report.total_files,
        report.total_symbols,
        report.public_symbols,
        if lang_summary.is_empty() {
            "none".to_string()
        } else {
            lang_summary
        }
    ));

    if !report.hubs.is_empty() {
        md.push_str("## Core Architecture Hubs\n\n");
        md.push_str("| Symbol | Kind | File | Degree |\n");
        md.push_str("| :--- | :--- | :--- | :--- |\n");
        for hub in &report.hubs {
            let path_str = hub.path.as_deref().unwrap_or("-");
            md.push_str(&format!(
                "| `{}` | {} | `{}` | {} |\n",
                hub.label, hub.kind, path_str, hub.degree
            ));
        }
        md.push('\n');
    }

    md.push_str("## Modules & Public APIs\n\n");

    for m in &report.modules {
        let dir_title = if m.directory == "." {
            "Root Directory (`.`)".to_string()
        } else {
            format!("Directory: `{}`", m.directory)
        };
        md.push_str(&format!("### {dir_title}\n\n"));

        for f in &m.files {
            md.push_str(&format!("#### `{}` ({})\n\n", f.path, f.language));

            for t in &f.types {
                let diag = if t.has_diagram { " `[diagram]`" } else { "" };
                md.push_str(&format!("- **`{}`**{}\n", t.signature, diag));
                if let Some(doc) = &t.doc {
                    for line in doc.lines() {
                        md.push_str(&format!("  > {line}\n"));
                    }
                }
                for method in &t.methods {
                    let m_diag = if method.has_diagram { " `[diagram]`" } else { "" };
                    md.push_str(&format!("  - `{}`{}\n", method.signature, m_diag));
                    if let Some(doc) = &method.doc {
                        for line in doc.lines() {
                            md.push_str(&format!("    > {line}\n"));
                        }
                    }
                }
            }

            if !f.functions.is_empty() {
                if !f.types.is_empty() {
                    md.push_str("- **Functions**:\n");
                    for func in &f.functions {
                        let f_diag = if func.has_diagram { " `[diagram]`" } else { "" };
                        md.push_str(&format!("  - `{}`{}\n", func.signature, f_diag));
                        if let Some(doc) = &func.doc {
                            for line in doc.lines() {
                                md.push_str(&format!("    > {line}\n"));
                            }
                        }
                    }
                } else {
                    for func in &f.functions {
                        let f_diag = if func.has_diagram { " `[diagram]`" } else { "" };
                        md.push_str(&format!("- `{}`{}\n", func.signature, f_diag));
                        if let Some(doc) = &func.doc {
                            for line in doc.lines() {
                                md.push_str(&format!("  > {line}\n"));
                            }
                        }
                    }
                }
            }
            md.push('\n');
        }
    }

    md
}
