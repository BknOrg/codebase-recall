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
    Java,
    Kotlin,
    Vue,
    Svelte,
    Go,
    /// Configuration, not code: indexed for its keys only, so a setting read in
    /// code can be traced to where it is defined.
    Toml,
}

/// Whether a stored language group names configuration rather than source.
/// Config files carry indexed keys for `search`, but no symbols, so they stay
/// out of the dependency graph where they would only add unconnected nodes.
pub fn is_config_group(group: &str) -> bool {
    matches!(group, "toml")
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
            "java" => Language::Java,
            "kt" | "kts" => Language::Kotlin,
            "vue" => Language::Vue,
            "svelte" => Language::Svelte,
            "go" => Language::Go,
            "toml" => Language::Toml,
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
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::Vue => "vue",
            Language::Svelte => "svelte",
            Language::Go => "go",
            Language::Toml => "toml",
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
            "kt" => self.group() == "kotlin",
            "vue" => self.group() == "vue",
            "svelte" => self.group() == "svelte",
            other => other == self.group(),
        }
    }
}
