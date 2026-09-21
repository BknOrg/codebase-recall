<!-- generated-by: gsd-doc-writer -->
# Configuration

`code-rcl` (crate `codebase-recall`) is configured mostly through command-line flags. It also reads a small set of environment variables to locate language servers, and `code-rcl init` writes a per-project `config.toml`.

## Environment variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `CODE_RCL_LSP_RUST` | Optional | Search `PATH` | Full path to the Rust language server executable used by `sync --precise`. |
| `CODE_RCL_LSP_PYTHON` | Optional | Search `PATH` | Full path to the Python language server (for example pyright). |
| `CODE_RCL_LSP_JAVA` | Optional | Search `PATH` | Full path to the Java language server launcher. Needs a JDK 17+. |
| `CODE_RCL_LSP_KOTLIN` | Optional | Search `PATH` | Full path to the Kotlin language server. |
| `CODE_RCL_LSP_TYPESCRIPT` | Optional | Search `PATH` | Full path to the TypeScript language server. |
| `CODE_RCL_LSP_JAVASCRIPT` | Optional | Search `PATH` | Full path to the JavaScript language server. |
| `CODE_RCL_LSP_GO` | Optional | Search `PATH` | Full path to the Go language server. |
| `CODE_RCL_TEST_PRECISE` | Optional | unset | Test-only. Set to `1` to run the ignored precise-resolution accuracy tests (`tests/resolve_accuracy.rs`). |

The `CODE_RCL_LSP_*` variables are defined in `src/precise/backend.rs`. If a variable is set but does not point to an existing file, the precise backend returns an error naming the variable. Unset it to fall back to a `PATH` search. When an override is used, it keeps the launch arguments of the matching candidate server (for example `--stdio` for pyright).

The tool also reads `PATH` and, on Windows, `PATHEXT` to locate servers. `HOME` (or `USERPROFILE` on Windows) is used by `code-rcl setup` to find the user profile directory for `--global` installs.

No `.env` file is used. Files named `.env*` are deliberately skipped by `dump`.

## Config file format

`code-rcl init` creates a `.code-rcl/` directory in the target project containing:

- `cache.db`: the SQLite graph cache.
- `config.toml`: a default project configuration (not overwritten unless `--force` is passed).

It also appends `.code-ctx/` to the project's `.gitignore` if not already present. <!-- VERIFY: init.rs adds `.code-ctx/` to .gitignore, but the cache directory constant is `.code-rcl`; the mismatch may be a bug, so confirm intended behavior -->

Default `config.toml`:

```toml
# code-rcl project configuration
schema_version = 1

[sync]
# Skip source files larger than this many KB.
max_file_kb = 512
# Languages to analyze.
languages = ["rust", "javascript", "typescript", "python", "java", "kotlin"]

[graph]
# Drop resolved edges below this confidence.
min_confidence = 0.4
# Include edges to external (npm / pypi / crate) modules.
include_external = false
```

Note: in the current source, `config.toml` is written by `init` but no code reads it back. Effective settings come from the command-line flags below.

## Required vs optional settings

Nothing is required at startup. Every setting has a default, and language server overrides are only consulted for `sync --precise`. Commands that need the cache (`sync`, `graph`, `impact`, and others) expect `.code-rcl/` to exist; run `code-rcl init` first.

## Defaults

Defaults defined in `src/cli.rs`:

| Command | Flag | Default |
|---------|------|---------|
| `dump` | path | `.` |
| `dump` | `-o`, `--output` | `codebase-context` |
| `dump` | `--max-size-kb` | `50` |
| `dump` | depth option | `2` |
| `init` | `--project` | `.` |
| `sync` | `--project` | `.` |
| `sync` | max file size (KB) | `512` |
| `graph` | `--project` | `.` |
| `graph` | direction | `both` |
| `graph` | `--kinds` | `imports,calls,contains,implements` |
| `graph` | `--depth` | `2` |
| `graph` | min confidence | `0.4` |
| `graph` | `--format` | `html` |
| `impact` | `--depth` | `2` |
| `impact` | direction | `both` |
| `impact` | `--kinds` | `calls,imports,references` |
| `path` | `--kinds` | `calls,imports,references` |
| `path` | `--direction` | `forward` |
| `path` | max depth | `8` |
| `search` | limit | `25` |
| `setup` | `--target` | `all` |

Run `code-rcl <command> --help` for the authoritative list of flags per command.

Files skipped by `dump` regardless of settings: binary files, media assets, lockfiles, minified bundles, and `.env*` files (see `src/dump/walker.rs`).

## Per-environment overrides

There is no built-in concept of development, staging, or production environments. To vary behavior per project, run `code-rcl init` in each project (each gets its own `.code-rcl/`), and use the `CODE_RCL_LSP_*` variables per shell or CI job to select different language server binaries.

## AI agent integration config

`code-rcl setup` writes agent configuration rather than reading it:

- `--workspace` (project scope) or `--global` (user scope: `~/.gemini/config` and `~/.claude.json`)
- `--target gemini|claude|all` (default `all`)
- `--instructions`, `--git-hook`, `--claude-hook` for optional extras; `--remove` reverses them
- `code-rcl mcp [--project <path>]` starts the MCP server for a project
