//! One active compilation context. Commands are parsed as data and never executed.
use crate::{analysis::Language, config::ProjectConfig};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct CompileCommand {
    pub language: Language,
    pub language_explicit: bool,
    pub includes: Vec<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct HeaderContext {
    pub language: Language,
    pub language_source: &'static str,
    pub clear: bool,
    pub includer: Option<PathBuf>,
}
pub struct BuildContext {
    pub directory: Option<PathBuf>,
    pub commands: BTreeMap<PathBuf, CompileCommand>,
    pub flags: Option<CompileCommand>,
    pub headers: BTreeMap<PathBuf, HeaderContext>,
    pub fingerprint: String,
}
pub fn absolute(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return crate::cache::utils::plain_path(&p);
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for part in path.components() {
        if part == std::path::Component::ParentDir {
            out.pop();
        } else if part != std::path::Component::CurDir {
            out.push(part);
        }
    }
    out
}
pub fn is_header(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "h" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "tpp"
    )
}
pub const HEADER_POLICY: &str = "header-language-2";
// Probe results describe syntax only. They never establish a precise build context.
fn header_probe(
    path: &Path,
    source: &str,
    db: Option<&crate::cache::CacheDb>,
    pending: &mut Vec<(String, String)>,
) -> Result<String> {
    let hash = blake3::hash(source.as_bytes()).to_hex().to_string();
    let key = format!(
        "header_probe:{}",
        blake3::hash(path.to_string_lossy().as_bytes()).to_hex()
    );
    let memo = format!("{key}:{HEADER_POLICY}:{hash}");
    let persisted = format!("{memo}:persisted");
    if let Some(result) = crate::operation::memo(&memo) {
        if db.is_some() && crate::operation::memo(&persisted).is_none() {
            pending.push((
                key,
                serde_json::json!({"policy":HEADER_POLICY,"source_hash":hash,"result":result})
                    .to_string(),
            ));
            crate::operation::memo_set(persisted, "true".into());
        }
        return Ok(result);
    }
    if let Some(db) = db
        && let Some(raw) = db.meta_get(&key)?
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
        && value["policy"] == HEADER_POLICY
        && value["source_hash"] == hash
        && let Some(result) = value["result"]
            .as_str()
            .filter(|r| matches!(*r, "c_clean" | "cpp_clean" | "both_error"))
    {
        crate::operation::memo_set(memo, result.into());
        crate::operation::memo_set(persisted, "true".into());
        return Ok(result.into());
    }
    let _timer = crate::operation::time("header_language_probe");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c::LANGUAGE.into())?;
    let c = parser
        .parse(source, None)
        .context("C header syntax probe cancelled")?;
    let result = if !c.root_node().has_error() {
        "c_clean"
    } else {
        crate::operation::check()?;
        parser.set_language(&tree_sitter_cpp::LANGUAGE.into())?;
        let cpp = parser
            .parse(source, None)
            .context("C++ header syntax probe cancelled")?;
        if cpp.root_node().has_error() {
            "both_error"
        } else {
            "cpp_clean"
        }
    };
    crate::operation::memo_set(memo, result.into());
    if db.is_some() {
        pending.push((
            key,
            serde_json::json!({"policy":HEADER_POLICY,"source_hash":hash,"result":result})
                .to_string(),
        ));
        crate::operation::memo_set(persisted, "true".into());
    }
    Ok(result.into())
}
impl BuildContext {
    pub fn load(project: &Path, cfg: &ProjectConfig) -> Result<Self> {
        Self::load_impl(project, cfg, None)
    }
    pub fn load_cached(
        project: &Path,
        cfg: &ProjectConfig,
        db: &crate::cache::CacheDb,
    ) -> Result<Self> {
        Self::load_impl(project, cfg, Some(db))
    }
    fn load_impl(
        project: &Path,
        cfg: &ProjectConfig,
        db: Option<&crate::cache::CacheDb>,
    ) -> Result<Self> {
        let mut pending_probes = Vec::new();
        let project = absolute(project);
        let mut hash = blake3::Hasher::new();
        hash.update(HEADER_POLICY.as_bytes());
        hash.update(serde_json::to_string(&cfg.c_family)?.as_bytes());
        hash.update(serde_json::to_string(&cfg.precise.clangd)?.as_bytes());
        let candidates = if let Some(p) = &cfg.c_family.compilation_database_dir {
            let p = absolute(&project.join(p));
            ensure!(
                p.is_dir(),
                "compilation_database_dir is not a directory: {}",
                p.display()
            );
            vec![p]
        } else {
            vec![project.clone(), project.join("build")]
        };
        let directory = candidates.into_iter().find(|p| {
            p.join("compile_commands.json").is_file() || p.join("compile_flags.txt").is_file()
        });
        ensure!(
            cfg.c_family.compilation_database_dir.is_none() || directory.is_some(),
            "compilation_database_dir has no compile_commands.json or compile_flags.txt"
        );
        let mut commands = BTreeMap::new();
        let mut flags = None;
        if let Some(dir) = &directory {
            hash.update(dir.to_string_lossy().as_bytes());
            let db = dir.join("compile_commands.json");
            if db.is_file() {
                let data = std::fs::read(&db)?;
                hash.update(&data);
                let rows: serde_json::Value =
                    serde_json::from_slice(&data).context("invalid compile_commands.json")?;
                for row in rows
                    .as_array()
                    .context("compile_commands.json must be an array")?
                {
                    let cwd = row
                        .get("directory")
                        .and_then(|v| v.as_str())
                        .context("compile command requires directory")?;
                    let cwd = absolute(&dir.join(cwd));
                    let file = row
                        .get("file")
                        .and_then(|v| v.as_str())
                        .context("compile command requires file")?;
                    let file = absolute(&cwd.join(file));
                    let args = if let Some(args) = row.get("arguments") {
                        args.as_array()
                            .context("arguments must be an array")?
                            .iter()
                            .map(|a| {
                                a.as_str()
                                    .map(str::to_owned)
                                    .context("arguments must contain strings")
                            })
                            .collect::<Result<Vec<_>>>()?
                    } else {
                        split_command(
                            row.get("command")
                                .and_then(|v| v.as_str())
                                .context("compile command requires arguments or command")?,
                        )?
                    };
                    let args = expand_response_files(&args, &cwd, &mut hash, 0)?;
                    ensure!(
                        !args.is_empty(),
                        "empty compile command for {}",
                        file.display()
                    );
                    if commands.contains_key(&file) {
                        anyhow::bail!(
                            "multiple compile commands for {}; select a database with one active configuration per file",
                            file.display()
                        );
                    }
                    commands.insert(
                        file.clone(),
                        command(
                            &args,
                            &cwd,
                            Language::from_path(&file).unwrap_or(Language::C),
                        ),
                    );
                }
            } else {
                let data = std::fs::read(dir.join("compile_flags.txt"))?;
                hash.update(&data);
                let text = std::str::from_utf8(&data).context("compile_flags.txt must be UTF-8")?;
                let args: Vec<_> = text
                    .lines()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect();
                let args = expand_response_files(&args, dir, &mut hash, 0)?;
                flags = Some(command(&args, dir, Language::C));
            }
        }
        // All applicable project fragments, plus the user's clangd configuration, affect answers.
        let files = crate::dump::walker::collect_source_files(&project)?;
        let mut configs: BTreeSet<PathBuf> = files
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n == ".clangd"))
            .map(|p| absolute(p))
            .collect();
        for parent in project.ancestors() {
            configs.insert(parent.join(".clangd"));
        }
        let user = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("clangd/config.yaml"))
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(|p| PathBuf::from(p).join("Library/Preferences/clangd/config.yaml"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
                .map(|p| p.join("clangd/config.yaml"))
        };
        if let Some(p) = user {
            configs.insert(p);
        }
        for file in configs {
            if file.is_file() {
                hash.update(file.to_string_lossy().as_bytes());
                hash.update(&std::fs::read(file)?);
            }
        }
        let mut includes: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        let mut sources = Vec::new();
        let mut header_sources = BTreeMap::new();
        for file in &files {
            let Some(lang) = Language::from_path(file) else {
                continue;
            };
            if !matches!(lang, Language::C | Language::Cpp) {
                continue;
            }
            let path = absolute(file);
            if !is_header(&path) {
                sources.push(path.clone());
            }
            let info = commands.get(&path).or(flags.as_ref());
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let deps = literal_includes(&text)
                .into_iter()
                .filter_map(|spec| {
                    resolve_include(
                        &path,
                        &spec,
                        info.map(|c| c.includes.as_slice()).unwrap_or(&[]),
                    )
                })
                .collect();
            if is_header(&path) {
                header_sources.insert(path.clone(), text);
            }
            includes.insert(path, deps);
        }
        let mut consumers: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
        for source in &sources {
            let mut stack = includes.get(source).cloned().unwrap_or_default();
            let mut seen = BTreeSet::new();
            let paths = commands
                .get(source)
                .or(flags.as_ref())
                .map(|c| c.includes.as_slice())
                .unwrap_or(&[]);
            while let Some(header) = stack.pop() {
                // Classification follows indexed project headers, never system/vendor trees.
                if !includes.contains_key(&header) || !seen.insert(header.clone()) {
                    continue;
                }
                consumers
                    .entry(header.clone())
                    .or_default()
                    .insert(source.clone());
                if let Ok(text) = std::fs::read_to_string(&header) {
                    stack.extend(
                        literal_includes(&text)
                            .into_iter()
                            .filter_map(|spec| resolve_include(&header, &spec, paths)),
                    );
                }
            }
        }
        let overrides = cfg
            .c_family
            .header_language_overrides
            .iter()
            .map(|r| {
                Ok((
                    globset::Glob::new(&r.pattern)?.compile_matcher(),
                    if r.language == "cpp" {
                        Language::Cpp
                    } else {
                        Language::C
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut headers = BTreeMap::new();
        for path in includes.keys().filter(|p| is_header(p)) {
            let rel = path
                .strip_prefix(&project)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let explicit = overrides
                .iter()
                .find(|(g, _)| g.is_match(&rel))
                .map(|(_, l)| *l)
                .or_else(|| commands.get(path).map(|c| c.language))
                .or_else(|| {
                    flags
                        .as_ref()
                        .filter(|c| c.language_explicit)
                        .map(|c| c.language)
                });
            let owners = consumers.get(path);
            let langs: BTreeSet<_> = owners
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    commands
                        .get(p)
                        .map(|c| c.language)
                        .or_else(|| {
                            flags
                                .as_ref()
                                .filter(|c| c.language_explicit)
                                .map(|c| c.language)
                        })
                        .or_else(|| Language::from_path(p))
                })
                .map(|l| l.group())
                .collect();
            let ext_cpp = Language::from_path(path) == Some(Language::Cpp);
            let mut language = explicit.unwrap_or(if ext_cpp || langs.contains("cpp") {
                Language::Cpp
            } else {
                Language::C
            });
            let clear = explicit.is_some() || ext_cpp || langs.len() == 1;
            let mut language_source = if overrides.iter().any(|(g, _)| g.is_match(&rel)) {
                "override"
            } else if commands.contains_key(path) {
                "compilation_entry"
            } else if explicit.is_some() {
                "compile_flags"
            } else if ext_cpp {
                "extension"
            } else if langs.len() == 1 {
                "includer"
            } else {
                "default"
            };
            if !clear
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("h"))
            {
                match header_probe(path, &header_sources[path], db, &mut pending_probes)?.as_str() {
                    "cpp_clean" => {
                        language = Language::Cpp;
                        language_source = "syntax_fallback";
                    }
                    "c_clean" => {
                        language = Language::C;
                        language_source = "syntax_c";
                    }
                    _ => {}
                }
            }

            let includer = owners
                .into_iter()
                .flatten()
                .find(|p| {
                    commands
                        .get(*p)
                        .map(|c| c.language)
                        .or_else(|| {
                            flags
                                .as_ref()
                                .filter(|c| c.language_explicit)
                                .map(|c| c.language)
                        })
                        .or_else(|| Language::from_path(p))
                        == Some(language)
                })
                .cloned();
            hash.update(rel.as_bytes());
            hash.update(language.group().as_bytes());
            hash.update(language_source.as_bytes());
            hash.update(&[u8::from(clear)]);
            if let Some(p) = &includer {
                hash.update(p.to_string_lossy().as_bytes());
            }
            headers.insert(
                path.clone(),
                HeaderContext {
                    language,
                    language_source,
                    clear,
                    includer,
                },
            );
        }
        if let Some(db) = db
            && !pending_probes.is_empty()
        {
            db.meta_set_many(&pending_probes)?;
        }
        Ok(Self {
            directory,
            commands,
            flags,
            headers,
            fingerprint: hash.finalize().to_hex().to_string(),
        })
    }
    pub fn language(&self, path: &Path) -> Option<Language> {
        let detected = Language::from_path(path)?;
        if !matches!(detected, Language::C | Language::Cpp) {
            return Some(detected);
        }
        let path = absolute(path);
        self.headers
            .get(&path)
            .map(|h| h.language)
            .or_else(|| self.commands.get(&path).map(|c| c.language))
            .or_else(|| {
                self.flags
                    .as_ref()
                    .filter(|c| c.language_explicit)
                    .map(|c| c.language)
            })
            .or_else(|| Language::from_path(&path))
    }
    pub fn validate_precise(
        &self,
        project: &Path,
        files: &[&crate::cache::models::FileRow],
    ) -> Result<()> {
        ensure!(
            self.directory.is_some(),
            "C/C++ precise requires compile_commands.json or compile_flags.txt; generate it, then run code-rcl sync --precise"
        );
        for file in files {
            let path = absolute(&project.join(&file.path));
            if let Some(h) = self.headers.get(&path) {
                ensure!(
                    h.clear,
                    "ambiguous header language for {}; configure c_family.header_language_overrides",
                    file.path
                );
                ensure!(
                    self.commands.contains_key(&path)
                        || self.flags.is_some()
                        || h.includer
                            .as_ref()
                            .is_some_and(|p| self.commands.contains_key(p)),
                    "no compilation context or configured includer for header {}",
                    file.path
                );
            } else {
                ensure!(
                    self.flags.is_some() || self.commands.contains_key(&path),
                    "no compile command for {}; update compilation database",
                    file.path
                );
            }
        }
        Ok(())
    }
    pub fn include_paths(&self, path: &Path) -> Vec<PathBuf> {
        let p = absolute(path);
        self.commands
            .get(&p)
            .or_else(|| {
                self.headers
                    .get(&p)
                    .and_then(|h| h.includer.as_ref())
                    .and_then(|p| self.commands.get(p))
            })
            .or(self.flags.as_ref())
            .map(|c| c.includes.clone())
            .unwrap_or_default()
    }
}
fn command(args: &[String], cwd: &Path, default: Language) -> CompileCommand {
    let mut language = if args.first().is_some_and(|a| a.contains("++")) {
        Language::Cpp
    } else {
        default
    };
    let mut includes = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-x" && i + 1 < args.len() {
            i += 1;
            language = if args[i].contains("++") {
                Language::Cpp
            } else {
                Language::C
            };
        } else if a.starts_with("-xc++")
            || a.starts_with("-std=c++")
            || a.starts_with("-std=gnu++")
            || a.starts_with("/std:c++")
            || a == "/TP"
        {
            language = Language::Cpp;
        } else if a.starts_with("-std=c") || a.starts_with("-std=gnu") || a == "/TC" {
            language = Language::C;
        } else if ["-I", "-isystem", "-iquote", "/I"].contains(&a.as_str()) && i + 1 < args.len() {
            i += 1;
            includes.push(absolute(&cwd.join(&args[i])));
        } else if let Some(p) = a
            .strip_prefix("-I")
            .or_else(|| a.strip_prefix("-isystem"))
            .or_else(|| a.strip_prefix("-iquote"))
            .or_else(|| a.strip_prefix("/I"))
            && !p.is_empty()
        {
            includes.push(absolute(&cwd.join(p)));
        }
        i += 1;
    }
    let language_explicit = args.iter().any(|a| {
        a == "-x"
            || a.starts_with("-x")
            || a.starts_with("-std=")
            || a.starts_with("/std:")
            || a == "/TP"
            || a == "/TC"
    }) || args.first().is_some_and(|a| a.contains("++"));
    CompileCommand {
        language,
        language_explicit,
        includes,
    }
}
pub fn literal_includes(source: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut pending = vec![tree.root_node()];
    let mut out = Vec::new();
    while let Some(node) = pending.pop() {
        if node.kind() == "preproc_include"
            && let Some(path) = node.child_by_field_name("path")
            && matches!(path.kind(), "string_literal" | "system_lib_string")
            && let Some(text) = source.get(path.byte_range())
        {
            out.push(text.to_owned());
        }
        pending.extend(node.named_children(&mut node.walk()));
    }
    out.reverse();
    out
}
pub fn resolve_include(importer: &Path, spec: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    let local = spec.starts_with('"');
    let name = spec.trim_matches(['"', '<', '>']);
    let mut roots = Vec::new();
    if local {
        roots.push(importer.parent()?.to_path_buf());
    }
    roots.extend_from_slice(paths);
    roots
        .into_iter()
        .map(|p| absolute(&p.join(name)))
        .find(|p| p.is_file())
}
fn expand_response_files(
    args: &[String],
    cwd: &Path,
    hash: &mut blake3::Hasher,
    depth: usize,
) -> Result<Vec<String>> {
    ensure!(
        depth <= 8,
        "compile command response files are cyclic or nested deeper than 8"
    );
    let mut out = Vec::new();
    for arg in args {
        if let Some(file) = arg.strip_prefix('@') {
            let path = absolute(&cwd.join(file));
            let data = std::fs::read(&path)
                .with_context(|| format!("reading response file {}", path.display()))?;
            hash.update(path.to_string_lossy().as_bytes());
            hash.update(&data);
            let text = std::str::from_utf8(&data).context("response file must be UTF-8")?;
            out.extend(expand_response_files(
                &split_command(text)?,
                cwd,
                hash,
                depth + 1,
            )?);
        } else {
            out.push(arg.clone());
        }
    }
    Ok(out)
}
fn split_command(text: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' if quote == Some(c) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(c),
            '\\' if chars.peek().is_some_and(|n| {
                *n == '"' || *n == '\'' || (quote.is_none() && n.is_whitespace())
            }) =>
            {
                word.push(chars.next().unwrap())
            }
            c if c.is_whitespace() && quote.is_none() => {
                if !word.is_empty() {
                    args.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(c),
        }
    }
    ensure!(quote.is_none(), "unclosed quote in compile command");
    if !word.is_empty() {
        args.push(word);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(dir: &Path, file: &str, src: &str) {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    #[test]
    fn mixed_headers_overrides_and_transitive_includes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "a.c", "#include \"shared.h\"\nint a(void){return 1;}");
        write(root, "b.cpp", "#include \"shared.h\"\nint b(){return 2;}");
        write(
            root,
            "shared.h",
            "#include <nested.h>\nstruct Shared { int value; };",
        );
        write(root, "include/nested.h", "int nested(void);");
        write(root, "compile_flags.txt", "-Iinclude\n");
        let cfg = ProjectConfig::default();
        let c = BuildContext::load(root, &cfg).unwrap();
        let shared = &c.headers[&absolute(&root.join("shared.h"))];
        assert_eq!(shared.language, Language::C);
        assert!(!shared.clear);
        assert!(!c.headers[&absolute(&root.join("include/nested.h"))].clear);
        let mut cfg = cfg;
        cfg.c_family
            .header_language_overrides
            .push(crate::config::HeaderLanguageOverride {
                pattern: "**/*.h".into(),
                language: "c".into(),
            });
        let next = BuildContext::load(root, &cfg).unwrap();
        assert_ne!(c.fingerprint, next.fingerprint);
        assert!(next.headers[&absolute(&root.join("shared.h"))].clear);
        assert_eq!(next.language(&root.join("shared.h")), Some(Language::C));
    }
    #[test]
    fn database_selection_paths_and_duplicate_commands() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "src/main.C", "int main(){return 0;}");
        let row = serde_json::json!({"directory":root,"file":"src/main.C","arguments":["clang++","-std=c++23","-I","include dir","-c","src/main.C"]});
        write(
            root,
            "build/compile_commands.json",
            &serde_json::json!([row.clone()]).to_string(),
        );
        let cfg = ProjectConfig::default();
        let c = BuildContext::load(root, &cfg).unwrap();
        assert_eq!(c.directory.as_ref().unwrap(), &root.join("build"));
        assert_eq!(c.language(&root.join("src/main.C")), Some(Language::Cpp));
        assert!(c.include_paths(&root.join("src/main.C"))[0].ends_with("include dir"));
        write(
            root,
            "build/compile_commands.json",
            &serde_json::json!([row.clone(), row]).to_string(),
        );
        assert!(BuildContext::load(root, &cfg).is_err());
        write(root, "compile_flags.txt", "-std=c17\n");
        assert_eq!(
            BuildContext::load(root, &cfg).unwrap().directory.unwrap(),
            absolute(root)
        );
    }
    #[test]
    fn commands_are_data_and_flags_configuration_changes_fingerprint() {
        assert_eq!(
            split_command(r#""C:\Program Files\LLVM\bin\clang++.exe" -I"include path" -DNAME=1"#)
                .unwrap()[0],
            r"C:\Program Files\LLVM\bin\clang++.exe"
        );
        assert!(split_command("clang \"unterminated").is_err());
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "main.c", "int main(void){return 0;}");
        write(root, "compile_flags.txt", "-std=c17\n-DMODE=1\n");
        let cfg = ProjectConfig::default();
        let first = BuildContext::load(root, &cfg).unwrap().fingerprint;
        write(root, "compile_flags.txt", "-std=c17\n-DMODE=2\n");
        let second = BuildContext::load(root, &cfg).unwrap().fingerprint;
        assert_ne!(first, second);
        write(root, ".clangd", "CompileFlags:\n  Add: [-DOTHER=1]\n");
        assert_ne!(second, BuildContext::load(root, &cfg).unwrap().fingerprint);
    }
    #[test]
    fn include_discovery_uses_syntax_and_ignores_comments() {
        assert_eq!(
            literal_includes("/*\n#include \"fake.h\"\n*/\n#include \"real.h\"\n"),
            vec!["\"real.h\""]
        );
    }
    #[test]
    fn explicit_shared_language_and_response_files_do_not_reclassify_other_files() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "only.h", "int value();");
        write(root, "args.rsp", "-std=c++23 -Iinclude");
        write(root, "compile_flags.txt", "@args.rsp\n");
        let cfg = ProjectConfig::default();
        let first = BuildContext::load(root, &cfg).unwrap();
        assert_eq!(first.language(&root.join("only.h")), Some(Language::Cpp));
        assert!(first.headers[&absolute(&root.join("only.h"))].clear);
        assert_eq!(
            first.language(&root.join("config.toml")),
            Some(Language::Toml)
        );
        assert_eq!(first.language(&root.join("README.md")), None);
        write(root, "args.rsp", "-std=c++23 -Iinclude -DCHANGED=1");
        assert_ne!(
            first.fingerprint,
            BuildContext::load(root, &cfg).unwrap().fingerprint
        );
    }
    fn reset_operation() {
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
    }
    #[test]
    fn uncertain_headers_probe_cache_and_evidence_precedence() {
        reset_operation();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Event.h",
            include_str!("../tests/fixtures/header_duplicate_app/Event.h"),
        );
        write(root, "plain.h", "struct Plain {int value;};");
        write(root, "broken.h", "struct { ???");
        let db = crate::cache::CacheDb::open(root).unwrap();
        let mut cfg = ProjectConfig::default();
        let context = BuildContext::load_cached(root, &cfg, &db).unwrap();
        let event = &context.headers[&absolute(&root.join("Event.h"))];
        assert_eq!(event.language, Language::Cpp);
        assert_eq!(event.language_source, "syntax_fallback");
        assert!(!event.clear);
        assert_eq!(context.language(&root.join("plain.h")), Some(Language::C));
        assert_eq!(
            context.headers[&absolute(&root.join("broken.h"))].language_source,
            "default"
        );
        assert!(
            crate::operation::timings()
                .get("header_language_probe")
                .is_some()
        );
        let fingerprint = context.fingerprint.clone();
        reset_operation();
        assert_eq!(
            BuildContext::load_cached(root, &cfg, &db)
                .unwrap()
                .fingerprint,
            fingerprint
        );
        assert!(
            crate::operation::timings()
                .get("header_language_probe")
                .is_none()
        );
        drop(db);
        reset_operation();
        let db = crate::cache::CacheDb::open(root).unwrap();
        BuildContext::load_cached(root, &cfg, &db).unwrap();
        assert!(
            crate::operation::timings()
                .get("header_language_probe")
                .is_none()
        );
        write(root, "compile_flags.txt", "-x c\n");
        let c = BuildContext::load_cached(root, &cfg, &db).unwrap();
        assert_eq!(c.language(&root.join("Event.h")), Some(Language::C));
        assert!(c.headers[&absolute(&root.join("Event.h"))].clear);
        cfg.c_family.header_language_overrides = vec![
            crate::config::HeaderLanguageOverride {
                pattern: "*.h".into(),
                language: "cpp".into(),
            },
            crate::config::HeaderLanguageOverride {
                pattern: "*.h".into(),
                language: "c".into(),
            },
        ];
        let cpp = BuildContext::load_cached(root, &cfg, &db).unwrap();
        assert_eq!(cpp.language(&root.join("Event.h")), Some(Language::Cpp));
        assert_eq!(
            cpp.headers[&absolute(&root.join("Event.h"))].language_source,
            "override"
        );
        std::fs::remove_file(root.join("compile_flags.txt")).unwrap();
        cfg.c_family.header_language_overrides.clear();
        write(root, "Event.h", "int now_c(void);");
        reset_operation();
        let c = BuildContext::load_cached(root, &cfg, &db).unwrap();
        assert_eq!(c.language(&root.join("Event.h")), Some(Language::C));
        assert_ne!(c.fingerprint, fingerprint);
        assert!(
            crate::operation::timings()
                .get("header_language_probe")
                .is_some()
        );
    }
    #[test]
    fn old_c_analyzer_requires_sync_and_preserves_other_languages_and_snapshots() {
        reset_operation();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Event.h",
            include_str!("../tests/fixtures/header_duplicate_app/Event.h"),
        );
        write(root, "lib.rs", "pub fn keep() {}\n");
        write(
            root,
            ".code-rcl/config.toml",
            r#"[c_family]
header_language_overrides=[{pattern="*.h",language="cpp"}]
"#,
        );
        let mut db = crate::cache::CacheDb::open(root).unwrap();
        crate::service::auto_sync(&mut db, root).unwrap();
        let header = db
            .all_files()
            .unwrap()
            .into_iter()
            .find(|f| f.path == "Event.h")
            .unwrap();
        let snapshot = db.source_snapshot(&header).unwrap();
        let rust_id = db
            .all_symbols()
            .unwrap()
            .iter()
            .find(|s| s.name == "keep")
            .unwrap()
            .id;
        db.meta_set("historical_baseline", "preserved").unwrap();
        db.meta_set("analyzer:Event.h", "cpp-semantic-2:old-context")
            .unwrap();
        db.meta_set("c_family_context", "old-header-policy")
            .unwrap();
        assert!(
            crate::health::ensure_current(&db)
                .unwrap_err()
                .to_string()
                .contains("sync")
        );
        reset_operation();
        let stats = crate::service::auto_sync(&mut db, root).unwrap();
        assert_eq!(stats.changed, 1);
        assert_eq!(stats.unchanged, 1);
        crate::health::ensure_current(&db).unwrap();
        let header = db
            .all_files()
            .unwrap()
            .into_iter()
            .find(|f| f.path == "Event.h")
            .unwrap();
        assert_eq!(db.source_snapshot(&header).unwrap(), snapshot);
        assert_eq!(
            db.all_symbols()
                .unwrap()
                .iter()
                .find(|s| s.name == "keep")
                .unwrap()
                .id,
            rust_id
        );
        assert_eq!(
            db.meta_get("historical_baseline").unwrap().as_deref(),
            Some("preserved")
        );
        assert_eq!(
            db.meta_get("schema_version").unwrap().as_deref(),
            Some("13")
        );
    }
}
