use std::path::Path;

/// A source language variant the analyzer understands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    JavaScript,
    Jsx,
    TypeScript,
    Tsx,
    Python,
}

impl Language {
    /// Detect from a file extension. Returns `None` for unsupported files.
    pub fn from_path(path: &Path) -> Option<Language> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Language::Rust,
            "js" | "mjs" | "cjs" => Language::JavaScript,
            "jsx" => Language::Jsx,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "py" | "pyi" => Language::Python,
            _ => return None,
        })
    }

    /// Coarse language group used for storage and `--language` filtering.
    pub fn group(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::JavaScript | Language::Jsx => "javascript",
            Language::TypeScript | Language::Tsx => "typescript",
            Language::Python => "python",
        }
    }

    /// Whether an analyzer backend exists for this language yet.
    pub fn is_supported(&self) -> bool {
        true
    }

    /// Match a user-supplied `--language` token against this variant's group.
    pub fn matches_filter(&self, token: &str) -> bool {
        let t = token.trim().to_ascii_lowercase();
        match t.as_str() {
            "js" => self.group() == "javascript",
            "ts" => self.group() == "typescript",
            "py" => self.group() == "python",
            other => other == self.group(),
        }
    }
}
