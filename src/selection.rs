//! Explicit selection is shared by lexical, semantic, hybrid and architecture reports.
use anyhow::Result;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(clap::Args, Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    #[arg(long = "language", value_delimiter = ',')]
    pub languages: Vec<String>,
    #[arg(long = "path")]
    pub paths: Vec<String>,
    #[arg(long = "exclude-path")]
    pub exclude_paths: Vec<String>,
    #[arg(long = "category", value_delimiter = ',')]
    pub categories: Vec<String>,
}
pub fn globs(values: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for value in values {
        anyhow::ensure!(
            !value.trim().is_empty()
                && !value.contains('\\')
                && !Path::new(value).is_absolute()
                && !value.split('/').any(|v| v == ".."),
            "scope globs must be nonempty project-relative forward-slash paths"
        );
        builder.add(GlobBuilder::new(value).literal_separator(true).build()?);
    }
    Ok(builder.build()?)
}
pub struct Selection {
    include: GlobSet,
    exclude: GlobSet,
    categories: GlobSet,
    has_categories: bool,
    languages: Vec<String>,
}
impl Selection {
    pub fn new(project: &Path, filters: &Filters) -> Result<Self> {
        let cfg = crate::config::ProjectConfig::load(project)?;
        let mut category_paths = vec![];
        for category in &filters.categories {
            category_paths.extend(
                cfg.scope
                    .categories
                    .get(category)
                    .ok_or_else(|| {
                        anyhow::anyhow!("category {category} has no configured scope globs")
                    })?
                    .clone(),
            );
        }
        Self::validate(filters)?;
        Ok(Self {
            include: globs(&filters.paths)?,
            exclude: globs(&filters.exclude_paths)?,
            categories: globs(&category_paths)?,
            has_categories: !filters.categories.is_empty(),
            languages: filters.languages.clone(),
        })
    }
    pub fn validate(filters: &Filters) -> Result<()> {
        for language in &filters.languages {
            anyhow::ensure!(
                [
                    "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx", "ipp", "tpp", "ixx", "cppm",
                    "c++", "h++", "rs", "js", "jsx", "ts", "tsx", "py", "kt", "go", "java", "vue",
                    "svelte", "toml"
                ]
                .iter()
                .filter_map(
                    |ext| crate::analysis::Language::from_path(Path::new(&format!("source.{ext}")))
                )
                .any(|l| l.matches_filter(language)),
                "unsupported language filter: {language}"
            );
        }
        globs(&filters.paths)?;
        globs(&filters.exclude_paths)?;
        anyhow::ensure!(
            filters.categories.iter().all(|v| !v.trim().is_empty()),
            "category must not be empty"
        );
        Ok(())
    }
    pub fn accepts(&self, path: &str) -> bool {
        (self.include.is_empty() || self.include.is_match(path))
            && !self.exclude.is_match(path)
            && (!self.has_categories || self.categories.is_match(path))
            && (self.languages.is_empty()
                || crate::analysis::Language::from_path(Path::new(path))
                    .is_some_and(|l| self.languages.iter().any(|f| l.matches_filter(f))))
    }
}
impl Filters {
    pub fn from_mcp(value: &serde_json::Value) -> Self {
        let list = |key| {
            value[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            languages: list("languages"),
            paths: list("paths"),
            exclude_paths: list("exclude_paths"),
            categories: list("categories"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_is_explicit_and_shared_across_code_categories() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".code-rcl")).unwrap();
        std::fs::write(
            d.path().join(".code-rcl/config.toml"),
            "[scope.categories]\nproduction=['src/**']\ntests=['tests/**']\n",
        )
        .unwrap();
        let filters = Filters {
            languages: vec!["rust".into()],
            paths: vec!["**/*.rs".into()],
            exclude_paths: vec!["src/generated/**".into()],
            categories: vec!["production".into()],
        };
        let scope = Selection::new(d.path(), &filters).unwrap();
        assert!(scope.accepts("src/lib.rs"));
        assert!(!scope.accepts("tests/test.rs"));
        assert!(!scope.accepts("src/generated/api.rs"));
        assert!(!scope.accepts("src/lib.py"));
        assert!(
            Selection::new(d.path(), &Filters::default())
                .unwrap()
                .accepts("tests/test.rs")
        );
        assert!(
            Selection::validate(&Filters {
                languages: vec!["bogus".into()],
                ..Default::default()
            })
            .is_err()
        );
    }
}
