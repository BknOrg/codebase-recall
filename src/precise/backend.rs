//! Which language server backs each language, and how to find it on this
//! machine. Nothing here is bundled: `--precise` uses the servers the user has
//! installed, and says exactly how to install a missing one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One language server, and everything needed to launch it and to explain
/// itself when it cannot be launched.
pub struct Backend {
    /// The `files.language` group this serves.
    pub lang_group: &'static str,
    /// LSP `languageId` for `textDocument/didOpen`.
    pub language_id: &'static str,
    /// Executables to try, in order, each with its own arguments.
    pub candidates: &'static [(&'static str, &'static [&'static str])],
    /// Environment variable holding an explicit path to the executable.
    pub env_override: &'static str,
    /// How to install the server, printed when none of `candidates` is found.
    pub install_hint: &'static str,
    /// Files that let the server build a real project model.
    pub project_markers: &'static [&'static str],
    /// What degrades when no marker is present.
    pub marker_hint: &'static str,
    /// How long to let the server index the project before querying it.
    pub index_timeout_secs: u64,
}

pub const BACKENDS: &[Backend] = &[
    Backend {
        lang_group: "rust",
        language_id: "rust",
        candidates: &[("rust-analyzer", &[])],
        env_override: "CODE_RCL_LSP_RUST",
        install_hint: "install it with `rustup component add rust-analyzer`, or download a release \
                       from https://github.com/rust-lang/rust-analyzer/releases",
        project_markers: &["Cargo.toml"],
        marker_hint: "rust-analyzer resolves names through the Cargo crate graph; without a \
                      Cargo.toml it cannot see how the files fit together and most references \
                      will come back unresolved",
        index_timeout_secs: 300,
    },
    Backend {
        lang_group: "python",
        language_id: "python",
        candidates: &[
            ("pyright-langserver", &["--stdio"]),
            ("basedpyright-langserver", &["--stdio"]),
        ],
        env_override: "CODE_RCL_LSP_PYTHON",
        install_hint: "install it with `npm install -g pyright` (or `pip install pyright`)",
        // Pyright analyses a plain folder of .py files perfectly well.
        project_markers: &[],
        marker_hint: "",
        index_timeout_secs: 300,
    },
    Backend {
        lang_group: "java",
        language_id: "java",
        candidates: &[("jdtls", &[])],
        env_override: "CODE_RCL_LSP_JAVA",
        install_hint: "install the Eclipse JDT Language Server \
                       (https://github.com/eclipse-jdtls/eclipse.jdt.ls) and put its `jdtls` \
                       launcher on PATH, or point CODE_RCL_LSP_JAVA at it. It needs a JDK 17+ \
                       on PATH as well",
        project_markers: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            ".classpath",
        ],
        marker_hint: "Eclipse JDT LS resolves types through the build classpath; without a \
                      pom.xml or build.gradle it falls back to syntax-only analysis and most \
                      cross-file calls will come back unresolved",
        index_timeout_secs: 600,
    },
    Backend {
        lang_group: "kotlin",
        language_id: "kotlin",
        candidates: &[("kotlin-language-server", &[])],
        env_override: "CODE_RCL_LSP_KOTLIN",
        install_hint: "download a release from \
                       https://github.com/fwcd/kotlin-language-server/releases, unzip it and put \
                       `server/bin/kotlin-language-server` on PATH, or point \
                       CODE_RCL_LSP_KOTLIN at it",
        project_markers: &[
            "build.gradle.kts",
            "build.gradle",
            "settings.gradle.kts",
            "settings.gradle",
            "pom.xml",
        ],
        marker_hint: "kotlin-language-server resolves types through the Gradle/Maven classpath; \
                      without a build script it falls back to syntax-only analysis and most \
                      cross-file calls will come back unresolved",
        index_timeout_secs: 600,
    },
    Backend {
        lang_group: "typescript",
        language_id: "typescript",
        candidates: &[
            ("typescript-language-server", &["--stdio"]),
            ("vtsls", &["--stdio"]),
        ],
        env_override: "CODE_RCL_LSP_TYPESCRIPT",
        install_hint: "install it with `npm install -g typescript-language-server typescript`",
        project_markers: &["tsconfig.json", "package.json"],
        marker_hint: "without a tsconfig.json or package.json, path aliases (paths/baseUrl) \
                      and cross-file definitions may not resolve properly",
        index_timeout_secs: 300,
    },
    Backend {
        lang_group: "javascript",
        language_id: "javascript",
        candidates: &[
            ("typescript-language-server", &["--stdio"]),
            ("vtsls", &["--stdio"]),
        ],
        env_override: "CODE_RCL_LSP_JAVASCRIPT",
        install_hint: "install it with `npm install -g typescript-language-server typescript`",
        project_markers: &["jsconfig.json", "package.json"],
        marker_hint: "without a jsconfig.json or package.json, module resolutions may be incomplete",
        index_timeout_secs: 300,
    },
];

/// Comma-separated language groups `--precise` can serve, for help text.
pub fn supported_languages() -> String {
    BACKENDS
        .iter()
        .map(|b| b.lang_group)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A located server executable, ready to spawn.
#[derive(Debug)]
pub struct Launcher {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// What to call this server in messages.
    pub name: String,
}

impl Launcher {
    pub fn command(&self) -> Command {
        let ext = self
            .program
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Windows cannot execute .cmd/.bat directly — and npm-installed servers
        // (pyright) and the JVM ones (jdtls, kotlin-language-server) ship
        // exactly that — so route them through the shell.
        if cfg!(windows) && matches!(ext.as_str(), "cmd" | "bat") {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&self.program).args(&self.args);
            cmd
        } else {
            let mut cmd = Command::new(&self.program);
            cmd.args(&self.args);
            cmd
        }
    }
}

impl Backend {
    /// Find this backend's executable: the `env_override` first, then PATH.
    ///
    /// The error is written to be actionable on its own — it names what was
    /// looked for and how to install it.
    pub fn locate(&self) -> Result<Launcher, String> {
        if let Some(raw) = std::env::var_os(self.env_override) {
            let path = PathBuf::from(&raw);
            if !path.is_file() {
                return Err(format!(
                    "{} is set to `{}`, but that is not an existing file.\n  \
                     Point it at the language server executable, or unset it to search PATH.",
                    self.env_override,
                    path.display()
                ));
            }
            // An override names the executable, so it keeps the args of the
            // candidate it stands in for (pyright needs `--stdio`).
            let args = self
                .candidates
                .iter()
                .find(|(name, _)| {
                    path.file_stem()
                        .and_then(OsStr::to_str)
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
                })
                .map(|(_, args)| *args)
                .unwrap_or(self.candidates.first().map(|(_, a)| *a).unwrap_or(&[]));
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or(self.lang_group)
                .to_string();
            return Ok(Launcher {
                program: path,
                args: args.iter().map(|s| s.to_string()).collect(),
                name,
            });
        }

        for (name, args) in self.candidates {
            if let Some(program) = which(name) {
                return Ok(Launcher {
                    program,
                    args: args.iter().map(|s| s.to_string()).collect(),
                    name: (*name).to_string(),
                });
            }
        }

        let tried: Vec<&str> = self.candidates.iter().map(|(n, _)| *n).collect();
        Err(format!(
            "no language server found on PATH for {} (looked for: {}).\n  \
             To fix: {}.\n  \
             Or set {} to the executable's full path.",
            self.lang_group,
            tried.join(", "),
            self.install_hint,
            self.env_override
        ))
    }

    /// `true` when the project has none of the build files this server needs to
    /// resolve types properly.
    pub fn lacks_project_model(&self, project: &Path) -> bool {
        !self.project_markers.is_empty()
            && !self
                .project_markers
                .iter()
                .any(|m| project.join(m).exists())
    }
}

/// Locate `name` on PATH, honouring Windows' PATHEXT so `.cmd`/`.bat` wrappers
/// are found the way a shell would find them.
fn which(name: &str) -> Option<PathBuf> {
    let raw = Path::new(name);
    if raw.components().count() > 1 {
        return raw.is_file().then(|| raw.to_path_buf());
    }

    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        Vec::new()
    };

    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(lang_group: &str) -> &'static Backend {
        BACKENDS
            .iter()
            .find(|b| b.lang_group == lang_group)
            .expect("backend is registered")
    }

    #[test]
    fn every_backend_is_reachable_and_self_describing() {
        for b in BACKENDS {
            assert!(!b.candidates.is_empty(), "{} has no command", b.lang_group);
            assert!(
                !b.install_hint.is_empty(),
                "{} must tell the user how to install it",
                b.lang_group
            );
            assert!(
                b.project_markers.is_empty() == b.marker_hint.is_empty(),
                "{} declares project markers but no explanation (or vice versa)",
                b.lang_group
            );
        }
    }

    #[test]
    fn missing_server_error_names_the_fix() {
        let backend = backend("rust");
        // A name nothing will ever ship, so this exercises the "not found" path
        // whether or not rust-analyzer is installed on the test machine.
        let fake = Backend {
            candidates: &[("code-rcl-no-such-server", &[])],
            ..*backend
        };
        let err = fake.locate().unwrap_err();
        assert!(err.contains("code-rcl-no-such-server"));
        assert!(err.contains("rustup component add rust-analyzer"));
        assert!(err.contains("CODE_RCL_LSP_RUST"));
    }
}
