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
| `CODE_RCL_RUN_BENCHMARKS` | Optional | unset | Set to `1` to run the opt-in search benchmark; see [BENCHMARKS.md](BENCHMARKS.md). |
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

[sync]
# Skip source files larger than this many KB.
max_file_kb = 512
# Restrict analysis to these languages. Omit (or leave empty) to analyze every
# supported language: rust, javascript, typescript, python, java, kotlin, vue,
# svelte, go, toml.
# languages = ["rust", "python"]

[graph]
# Drop resolved edges below this confidence (0.0 to 1.0).
min_confidence = 0.3
# Include edges to external (npm / pypi / crate) modules.
include_external = false
# Cap on total graph nodes (0 disables the cap).
max_nodes = 4000
# BFS depth around --focus.
depth = 2
# Edge kinds to include.
kinds = ["imports", "calls", "contains", "implements"]

[embeddings]
model = "embeddinggemma-300m-q4"
dimensions = 768
batch_size = 4
threads = 0 # auto; manual override 1..64
batching = "file"
chunk_profile = "full"

[precise]
# true acts like passing --precise for sync, graph and serve.
enabled = false
# Opt-in verification of uncertain/risky references; mutually exclusive with enabled.
selective = false
selective_budget_secs = 30
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
| `[graph] min_confidence` | `0.3` | `graph`, `serve`, `dump -r` |
| `[graph] include_external` | `false` | `graph`, `serve`, `dump -r` |
| `[graph] max_nodes` | `4000` | `graph`, `serve` |
| `[graph] depth` | `2` | `graph`, `serve` (BFS depth around `--focus`) |
| `[graph] kinds` | `imports, calls, contains, implements` | `graph`, `serve`; `--kinds` replaces the list entirely |
| `[precise] enabled` | `false` | CLI `sync`, `graph`, `serve`; MCP sync/graph when precise is omitted and no_sync is false |
| `[precise] timeout_secs` | `15` | wherever a precise pass runs; `--precise-timeout` overrides it |
| `[precise.kotlin] server` | `jetbrains` | the Kotlin backend of a precise pass |
| `[storage] backend` | `bkndb` | the storage engine; `bkndb` is the only valid value |

Internal analysis commands (`impact`, `path`, `explain`, `report`, `digest` and the MCP graph/impact tools) ignore `[graph]` and keep `min_confidence` 0.0, so their output does not change when you edit `[graph]`. They do use `[sync]` (an implicit sync must use the same limits as `sync`).

**Kotlin language server.** `[precise.kotlin] server` picks the backend: `jetbrains` (default, `intellij-server --stdio`), `fwcd` (`kotlin-language-server`) or `auto` (`intellij-server`, falling back to `kotlin-language-server`). `CODE_RCL_LSP_KOTLIN` still overrides the binary path. Other languages are unaffected. Config only selects among these built-in servers; it never supplies a command line.

**Errors.** Unknown keys, out-of-range or invalid values (`min_confidence` outside 0.0 to 1.0, `max_file_kb` below 1, `timeout_secs` outside 1 to 86400, an unknown Kotlin server or language) and broken TOML are errors that name the config file, the key/value and the valid options. Config is never silently ignored.

**Older configs.** A `config.toml` written by an older `init` carries `languages = ["rust", "javascript", "typescript", "python", "java", "kotlin"]`. That list is now honored and restricts sync to those languages (go, vue, svelte and toml are skipped). Delete the line to analyze everything; `sync` prints a note on stderr when the filter comes from config.

**Security.** `[precise] enabled = true` is equivalent to typing `--precise` (language servers such as rust-analyzer may run project build scripts). `init` adds `.code-rcl/` to `.git/info/exclude` so the config does not travel with a clone.

## Required vs optional settings

Configuration is optional. Commands that need the cache create it if necessary; `init` also writes a default configuration and adds the local Git exclusion. Language-server overrides are consulted when a precise pass runs, including graph/impact requests. MCP servers without `--project` require an explicit project in each tool call.

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
| `search` | `--match-mode` | `substring` (also exact/prefix) |
| `mcp` | `--project` | unset; per-call project required |
| `setup` | `--target` | `all` |

Run `code-rcl <command> --help` for the authoritative list of flags per command.

Files skipped by `dump` regardless of settings: binary files, media assets, lockfiles, minified bundles, and `.env*` files (see `src/dump/walker.rs`).

## Per-environment overrides

There is no built-in concept of development, staging, or production environments. To vary behavior per project, run `code-rcl init` in each project (each gets its own `.code-rcl/`), and use the `CODE_RCL_LSP_*` variables per shell or CI job to select different language server binaries.

## AI agent integration config

`code-rcl setup` writes agent configuration rather than reading it:

- `--workspace` (project scope) or `--global` (native user locations for Codex, Antigravity, and Claude Code; see the MCP guide)
- `--target codex|antigravity|gemini|claude|all` (default `all`)
- `--instructions`, `--git-hook`, `--claude-hook` for optional extras; `setup --remove` reverses only these optional extras; `uninstall` removes skill/MCP and related extras
- `code-rcl mcp [--project <path>]` starts the MCP server for a project

## Cache correctness and precise failures

Normal sync reads/hashes eligible files even if their timestamp and size match.
Only changed contents are parsed. A change to analyzed files advances the cache
generation and invalidates precise answers across the project. Schema upgrades
rebuild derived tables while preserving config.toml and saved reports.
Recognized corruption/incompatible storage is quarantined before recreation;
permission, lock, and other I/O errors are reported rather than deleting state.

Precise requests fail explicitly on missing/failing servers or incomplete
indexing. `--precise --allow-heuristic-fallback` permits partial heuristic output.
MCP uses `precise: true` and `allow_heuristic_fallback: true`. This fallback is a
request option, not a config.toml key. Inspect warnings/status in JSON results.
The timeout bounds initialization and requests, and also limits the indexing
wait together with the backend's indexing budget.

## MCP limits and result format

MCP uses protocol 2025-06-18. The common max_output_kb argument defaults to 256
and accepts 1–2048 KiB; it is not a config.toml setting. MCP graph max_nodes
defaults to 2000, separate from the CLI graph default of 4000. Dump's per-file
max_size_kb defaults to 50. Do not confuse these three limits.

The removed json MCP argument is rejected. Results contain a schema_version=1
envelope in structuredContent and serialized text. See [MCP.md](MCP.md) for the
complete parameter table and [USAGE.md](USAGE.md#json-results) for metadata.

Workspace setup pins its absolute project root and copies the full skill bundle.
Global setup is opt-in; it leaves the server root unset. Built-in MCP setup targets
are Claude and Gemini/Antigravity; other clients need their own config format.

## Local embeddings (0.9.1)

```toml
[embeddings]
model = "embeddinggemma-300m-q4"
# model_dir = "/absolute/models/embeddinggemma-1"
dimensions = 768
batch_size = 4
threads = 0 # auto; manual override 1..64
batching = "file"
chunk_profile = "full"
```

Dimensions accepts 768 or 256; batch_size 1–32; threads 0 (auto) or 1–64. Unknown keys and
unsupported models are rejected. A relative model_dir resolves against the
project. Without it, model files live in the OS user cache under code-rcl/models.
`embeddings setup` saves a verified absolute directory while retaining unrelated
configuration. Dimension/chunk-format changes require embeddings sync --full.
`batching` accepts `file` (default) or `length`, without changing index identity.
Length groups up to eight files, 128 pending inputs and a 64 MiB scheduler buffer;
native model/parser memory is additional. It never runs parallel native batches.
`chunk_profile` defaults to `full`. `compact` remains a quality-gated harness
experiment and requires explicit experimental opt-in plus `sync --full`.
See [indexing benchmarks](INDEXING-BENCHMARK.md) before changing performance defaults.
Source [sync] language and size filters also constrain embedding scope.
The cache schema is now 12. Schemas 10/11 migrate additively, preserving valid
vectors and analysis; incompatible older layouts are rebuilt while retaining
config, reports and model files.
See [EMBEDDINGS.md](EMBEDDINGS.md) for indexing/freshness semantics.

Search uses the existing EmbeddingGemma model only. Legacy `[search] ranking_model_dir`
settings are accepted for configuration compatibility and ignored. No additional
model setup is required. See [automatic search](AUTOMATIC-SEARCH.md).

## Explicit code categories

```toml
[scope.categories]
production = ["src/**", "crates/*/src/**"]
tests = ["tests/**", "crates/*/tests/**"]
generated = ["bindings/generated/**"]
```

These are user-selected glob categories, not automatic classifications. Search
and report default to all source; choose `--category production` explicitly.
Multiple categories are ORed, then intersected with language/path filters;
exclude-path always removes matches. Empty/unknown categories and invalid globs
are errors. Globs are case-sensitive project-relative forward-slash paths; `*`
does not cross directories, `**` does. Scope filters affect retrieval/reporting,
not source-sync language coverage or which files the user has explicitly indexed.

### CPU thread budget

`[embeddings] threads = 0` (the default) resolves the logical CPUs available to
the process, falling back to one if detection fails. It applies to indexing
and query inference. A manual value from 1 to 64 remains supported; existing
projects with an explicit value keep that override. Automatic detection has
no four-thread or 64-thread cap. Batch size remains four by default.

Status/progress expose `threads_configured` and `threads_effective`. The
configured value stays unchanged on disk; sessions are cached by effective
thread count. Changing only threads does not invalidate vectors or require
full reindexing. It changes the CPU budget, not a guarantee of 100% utilization
or a speedup on every workload.

## C/C++ build context

`CODE_RCL_LSP_CLANGD` selects clangd for both `c` and `cpp`. Optional `[c_family]`
settings are `compilation_database_dir` and ordered `header_language_overrides`
(`pattern`, `language` = `c` or `cpp`). `[precise.clangd] query_drivers` defaults
to an empty allowlist. See [C/C++ configuration and freshness](C-CPLUSPLUS.md).


Selective LSP verification is opt-in: `sync --precise-selective` (MCP
`precise_selective: true`). See [selective precise mode](PRECISE-SELECTIVE.md) for coverage, backend setup, deadlines and cache recovery.
