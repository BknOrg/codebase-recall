# Usage guide — 0.10.0

Run commands from the target project, or pass `--project /absolute/path` to
commands that support it. Examples use `CacheDb` as a placeholder symbol; search
for a real symbol before substituting it. Use `code-rcl COMMAND --help` for the
complete flag list.

## Choose the command for your question

| Task | Command |
| --- | --- |
| Understand architecture | `code-rcl report` |
| Inspect declarations in a module | `code-rcl digest src --project .` |
| Find a known symbol | `code-rcl search CacheDb --match-mode exact` |
| Find names starting with a prefix | `code-rcl search Cache --match-mode prefix` |
| Search names/signatures by substring | `code-rcl search cache` |
| Search call-argument strings | `code-rcl search cache_dir --strings` |
| Understand a symbol | `code-rcl explain CacheDb` |
| Inspect callers and callees | `code-rcl impact CacheDb --depth 2` |
| Inspect tracked working-tree changes | `code-rcl impact --diff` |
| Find a connection | `code-rcl path main parse --direction any` |
| Bundle related source | `code-rcl dump -r CacheDb --depth 2 -o context.md` |
| Export a graph | `code-rcl graph --format json -o graph.json` |
| Open the viewer | `code-rcl serve` |

Exact/prefix search uses an ASCII-normalized name index. Substring is the
default and also searches signatures; fuzzy suggestions and text fallback help
when no matching declaration is found. `--limit` controls declaration results.
See [benchmarks](BENCHMARKS.md) for measured costs.

## Before and after an edit

```sh
code-rcl search parse --match-mode prefix
code-rcl explain parse_config
code-rcl impact parse_config --direction reverse --depth 2
# Read relevant source, make the edit, and run the project's tests.
code-rcl impact --diff --json
```

Diff compares HEAD against tracked working-tree contents, including staged and
unstaged changes. Deleted definitions use the HEAD graph; modified definitions
combine relationships from both sides. `change_kind` identifies added,
modified, and deleted targets. Renames are deletion/addition pairs. Untracked
files are excluded; stage intended new files with Git if they should participate.
The HEAD graph uses AST heuristics and does not run historical build scripts.
Diff reports omit isolated targets and retain the ten most affected; inspect
metadata for the omitted count.

## JSON results

Use `--json` for sync, search, digest, impact, path, explain, and report. Graph
uses `--format json`; `-o -` sends that JSON to stdout:

```sh
code-rcl search CacheDb --match-mode exact --json
code-rcl sync --json
code-rcl graph --format json -o -
```

CLI JSON and MCP structured results share an envelope. This illustrative empty
search result shows the shape; paths, counts, and values depend on the project:

```json
{
  "schema_version": 1,
  "status": "ok",
  "data": {
    "query": "UnknownSymbol",
    "total_matches": 0,
    "exact_symbols": [],
    "string_literals": [],
    "fuzzy_suggestions": [],
    "grep_matches": []
  },
  "metadata": {
    "project": "/absolute/path/to/project",
    "cache_generation": 1,
    "resolution_mode": "heuristic",
    "freshness": "verified",
    "warnings": [],
    "truncated": false,
    "omitted_count": 0
  }
}
```

Read former root-level arrays/fields through `data`. For example, impact returns
an array under `data`, and graph nodes are under `data.nodes`. Graph payloads
retain `data.version: 2`; the envelope version is independently `schema_version: 1`.

- `status`: `ok` or `partial`; MCP execution failures return `error` and
  `isError: true`. CLI failures use a nonzero exit code and stderr.
- `freshness`: `verified` after refresh, or `unverified` with `--no-sync`.
  Freshness concerns the analyzed scope, not complete parser coverage.
- `resolution_mode`: heuristic, cached precise, or mixed results. MCP can also
  report `precise` for a requested successful pass.
- `warnings`: skipped/oversized files, parser issues, or resolution failures.
- `truncated` / `omitted_count`: results were limited; narrow the query or raise
  its limits before treating absence as evidence.

An empty graph/path/impact result is not proof that no dependency exists.
Heuristic confidence is a ranking signal, not a calibrated probability.

## Precise resolution and explicit fallback

```sh
code-rcl sync --precise --language rust --precise-timeout 30
code-rcl impact CacheDb --precise
code-rcl sync --precise --allow-heuristic-fallback --json
```

Install the relevant language servers separately. Missing servers, initialization
failure, definition-request failure, and incomplete indexing fail a precise
request. Explicit fallback allows partial heuristic results with warnings.
`--precise-timeout` applies to initialization/requests and bounds the indexing
wait together with the backend limit.

Changing analyzed content invalidates the project's precise results. Normal
sync hashes files even when size and timestamp are unchanged. Use
`--precise-full` with `--precise` to deliberately repeat the pass without edits.

## Viewer and exports

```sh
code-rcl serve --no-open --port 8080
code-rcl graph --format html,json,dot -o build/graph
```

The viewer prints a loopback URL and uses a session token for API requests.
Export actions use POST. Blank output names receive unique names; an explicit
existing filename returns HTTP 409, so choose a different filename. Traversal
and links outside the project are rejected. Static HTML exports work without
the local server; server export actions are unavailable after it stops.

For direct integrations with viewer endpoints, `/dump`, `/impact`, and `/quit`
require POST plus `X-Code-Rcl-Token`. `/source` requires the same token on GET;
`/live` uses its token query parameter. Host and Origin must match the local
server. Prefer the built-in viewer rather than hard-coding a session token.

## Update an installed skill or MCP server

Changing source files does not replace an installed executable or global skill.
From the code-rcl checkout, install the current binary:

```sh
cargo install --path . --locked
code-rcl --version
```

Then run `code-rcl setup --workspace` in the project you want to analyze.
This copies the embedded skill bundle and writes supported workspace MCP
configuration. Global setup is explicit through `--global`.

To update only a skill copy in a chosen directory, PowerShell supports:

```powershell
code-rcl setup --skill-only --skill-dir "$env:USERPROFILE\.agents\skills\code-rcl"
```

This updates the bundle at that path without changing MCP configuration.
Restart/reconnect the agent client afterward. Use [the MCP guide](MCP.md) for
client configuration and [the migration guide](MIGRATION-0.9.md) for breaking changes.

## Semantic and hybrid retrieval

See the [embedding usage guide](EMBEDDINGS.md) for explicit model setup, indexing,
semantic/hybrid search, partial results, build variants and troubleshooting.

Use `--lexical` for model-free name/content lookup. Normal semantic/hybrid search
refreshes pending embeddings after source sync. Result groups share a single
limit and expose `match_counts`. See [automatic search](AUTOMATIC-SEARCH.md) for
qualified names, snapshot consistency and the gated feature-retrieval pipeline.

For progress, Ctrl+C/resume, status during indexing, coverage diagnostics and
response budgets, see [operations](OPERATIONS.md). Search and report accept
`--language`, `--path`, `--exclude-path`, and configured `--category` filters.

### Explicit length batching

In `.code-rcl/config.toml`, set `[embeddings].batching = "length"` to group
embedding inputs across files. Keep `chunk_profile = "full"` for ordinary use.
Run `code-rcl embeddings sync`; the same setting applies to MCP embedding sync.
Switching batching does not need `--full` and does not reduce source coverage.
Existing complete indexes do no inference. See [benchmarking](INDEXING-BENCHMARK.md)
for cold-run comparisons and the separate experimental compact profile.
