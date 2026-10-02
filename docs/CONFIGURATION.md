<!-- generated-by: gsd-doc-writer -->
# Configuration

`code-rcl` (crate `codebase-recall`) is configured mostly through command-line flags. It also reads a small set of environment variables to locate language servers, and `code-rcl init` writes a per-project `config.toml`.

## Environment variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `CODE_RCL_LSP_RUST` | Optional | Search `PATH` | Full path to the Rust language server executable used by `sync --precise`. |
| `CODE_RCL_LSP_PYTHON` | Optional | Search `PATH` | Full path to the Python language server (for example pyright). |
| `CODE_RCL_LSP_JAVA` | Optional | Search `PATH` | Full path to the Java language server launcher. Needs a JDK 17+. |
| `CODE_RCL_LSP_KOTLIN` | Optional | Search `PATH` | Full path to the Kotlin language server: JetBrains' `intellij-server` (default) or fwcd's `kotlin-language-server`, whichever `[precise.kotlin] server` selects. Overrides the binary path only, not the selection. |
| `CODE_RCL_LSP_TYPESCRIPT` | Optional | Search `PATH` | Full path to the TypeScript language server. |
| `CODE_RCL_LSP_JAVASCRIPT` | Optional | Search `PATH` | Full path to the JavaScript language server. |
| `CODE_RCL_LSP_GO` | Optional | Search `PATH` | Full path to the Go language server. |
| `CODE_RCL_TEST_PRECISE` | Optional | unset | Test-only. Set to `1` to run the ignored precise-resolution accuracy tests (`tests/resolve_accuracy.rs`). |

The `CODE_RCL_LSP_*` variables are defined in `src/precise/backend.rs`. If a variable is set but does not point to an existing file, the precise backend returns an error naming the variable. Unset it to fall back to a `PATH` search. When an override is used, it keeps the launch arguments of the matching candidate server (for example `--stdio` for pyright).

The tool also reads `PATH` and, on Windows, `PATHEXT` to locate servers. `HOME` (or `USERPROFILE` on Windows) is used by `code-rcl setup` to find the user profile directory for `--global` installs.

No `.env` file is used. Files named `.env*` are deliberately skipped by `dump`.

## Config file format

`code-rcl init` creates a `.code-rcl/` directory in the target project containing:

- `cache.bkndb`: the bkndb graph cache. It is derived from your sources, so it is safe to delete; the next sync rebuilds it. A `cache.db` left by an older version is no longer read and can be deleted.
- `config.toml`: a default project configuration (not overwritten unless `--force` is passed).

In a git repository it also appends `.code-rcl/` to `.git/info/exclude` (if not already present), so the cache is never committed and your `.gitignore` is left alone.

Default `config.toml` (written by `code-rcl init`; every line is optional):

```toml
# code-rcl project configuration
# Precedence: command-line flag > this file > built-in default.
# Every key is optional; a missing key uses the built-in default shown here.
schema_version = 1

[storage]
# Storage engine for .code-rcl/cache.bkndb. "bkndb" is the only supported value.
backend = "bkndb"

[sync]
# Skip source files larger than this many KB.
max_file_kb = 512
# Restrict analysis to these languages. Omit (or leave empty) to analyze every
# supported language: rust, javascript, typescript, python, java, kotlin, vue,
# svelte, go, toml.
# languages = ["rust", "python"]

[graph]
# Drop resolved edges below this confidence (0.0 to 1.0).
min_confidence = 0.4
# Include edges to external (npm / pypi / crate) modules.
include_external = false
# Cap on total graph nodes (0 disables the cap).
max_nodes = 4000
# BFS depth around --focus.
depth = 2
# Edge kinds to include.
kinds = ["imports", "calls", "contains", "implements"]

[precise]
# true acts like passing --precise for sync, graph and serve.
enabled = false
# Seconds to wait for a single language-server answer.
timeout_secs = 15

[precise.kotlin]
# Kotlin language server: "jetbrains" (intellij-server, default),
# "fwcd" (kotlin-language-server) or "auto" (jetbrains, falling back to fwcd).
server = "jetbrains"
```

### How config.toml is applied

Precedence is always **command-line flag > `config.toml` > built-in default**. A missing file, or a missing key, silently uses the built-in default, so projects without a config (or with an older one) behave as before. An explicit flag always wins, including `--include-external=false` over `include_external = true` and `--max-nodes 0` over a non-zero `max_nodes`.

| Key | Default | Honored by |
|-----|---------|------------|
| `[sync] max_file_kb` | `512` | `sync` and every implicit sync (`graph`, `serve`, `dump`, `impact`, MCP, ...), so the cache never flips between two size limits |
| `[sync] languages` | all | same as above; `--language` overrides it |
| `[graph] min_confidence` | `0.4` | `graph`, `serve`, `dump -r` |
| `[graph] include_external` | `false` | `graph`, `serve`, `dump -r` |
| `[graph] max_nodes` | `4000` | `graph`, `serve` |
| `[graph] depth` | `2` | `graph`, `serve` (BFS depth around `--focus`) |
| `[graph] kinds` | `imports, calls, contains, implements` | `graph`, `serve`; `--kinds` replaces the list entirely |
| `[precise] enabled` | `false` | `sync`, `graph`, `serve` only (acts like `--precise`) |
| `[precise] timeout_secs` | `15` | wherever a precise pass runs; `--precise-timeout` overrides it |
| `[precise.kotlin] server` | `jetbrains` | the Kotlin backend of a precise pass |
| `[storage] backend` | `bkndb` | the storage engine; `bkndb` is the only valid value |

Internal analysis commands (`impact`, `path`, `explain`, `report`, `digest` and the MCP graph/impact tools) ignore `[graph]` and keep `min_confidence` 0.0, so their output does not change when you edit `[graph]`. They do use `[sync]` (an implicit sync must use the same limits as `sync`).

**Kotlin language server.** `[precise.kotlin] server` picks the backend: `jetbrains` (default, `intellij-server --stdio`), `fwcd` (`kotlin-language-server`) or `auto` (`intellij-server`, falling back to `kotlin-language-server`). `CODE_RCL_LSP_KOTLIN` still overrides the binary path. Other languages are unaffected. Config only selects among these built-in servers; it never supplies a command line.

**Errors.** Unknown keys, out-of-range or invalid values (`min_confidence` outside 0.0 to 1.0, `max_file_kb` below 1, `timeout_secs` outside 1 to 86400, an unknown Kotlin server or language) and broken TOML are errors that name the config file, the key/value and the valid options. Config is never silently ignored.

**Older configs.** A `config.toml` written by an older `init` carries `languages = ["rust", "javascript", "typescript", "python", "java", "kotlin"]`. That list is now honored and restricts sync to those languages (go, vue, svelte and toml are skipped). Delete the line to analyze everything; `sync` prints a note on stderr when the filter comes from config.

**Security.** `[precise] enabled = true` is equivalent to typing `--precise` (language servers such as rust-analyzer may run project build scripts). `init` adds `.code-rcl/` to `.git/info/exclude` so the config does not travel with a clone.

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
