# CLI reference - 0.9.1

Discover parameters with `code-rcl COMMAND --help`.

```sh
code-rcl report --json
code-rcl digest src --project . --json
code-rcl search CacheDb --json
code-rcl impact CacheDb --depth 2 --json
code-rcl impact --diff --json
code-rcl path main parse --max-depth 8 --json
code-rcl explain CacheDb --json
code-rcl dump -r CacheDb --depth 2 -o context.md
code-rcl graph --format json -o graph.json
code-rcl sync --precise-selective
code-rcl sync --precise-selective --precise-full
code-rcl explain CacheDb --precise-selective --json
code-rcl sync --precise
```

JSON results use `schema_version`, `status`, `data`, and `metadata`. Former
root-level arrays/fields are inside `data`. Envelope `schema_version` is 2;
graph data uses `version: 3`.
Impact depth is 1-8, path 1-20, focused dump/graph 1-5 for MCP.
`--allow-heuristic-fallback` explicitly permits partial precise results.
`--no-sync` skips refresh and marks freshness unverified, but rejects stale semantic
generations. The cache is `.code-rcl/cache.bkndb`; schemas 10–12 migrate to 13,
preserving source snapshots and invalidating derived analysis/search/vectors.
Run normal source sync with the upgraded build; analyzer revisions trigger reparse.
See [symbol semantics](symbols.md) for identity, uncertainty and migration behavior.

```sh
code-rcl graph --kinds calls,inherits,overrides --min-confidence 0.5 --format json -o graph.json
```

CLI graph defaults to minimum confidence 0.3 unless configured otherwise.
Ambiguous guesses are capped at 0.35; inspect edge resolution evidence/alternatives
and resolution_summary before treating a result as a dependency.

Search supports `--match-mode exact|prefix|substring` (MCP: `match_mode`); exact and prefix use the normalized-name index. Default is substring.

`--lexical` forces lexical lookup. Lexical result groups share one output limit;
`total_matches` counts eligible items before that limit, with `match_counts`
per group. Automatic task retrieval remains release-gated; see embeddings.md.

For semantic/hybrid retrieval and explicit model/index setup in 0.9.1, read
[embeddings.md](embeddings.md). Source and embedding freshness are separate.

Long embedding syncs: --progress auto|plain|off controls stderr rendering; JSON
stdout remains one final envelope. embeddings status --files explains blocked
files and --path PATH reports an explicit ignore rule. Status under a cache lock
uses a saved snapshot with unverified coverage. First Ctrl+C cancels after the
current native call; rerun ordinary sync to resume committed work.
Search/report share --language, --path, --exclude-path and configured --category.

Embedding batching is project configuration: `[embeddings].batching="file"`
(default) or `"length"`. Switching it needs no full rebuild. Keep
`chunk_profile="full"` for ordinary work; compact remains a rejected production
experiment after class-query regression. See [embeddings](embeddings.md) for
setup, counters and recovery. Batching speed does not imply fewer scoped files.

## Agent integration lifecycle

```sh
code-rcl setup --workspace --target codex
code-rcl setup --global --target all
code-rcl uninstall --workspace --target codex
code-rcl uninstall --global --target all
```

Use the same command for Codex, Claude Code, and Antigravity/Gemini. Uninstall
removes related managed extras too; component selectors restrict removal.
Shared and unknown legacy ownership is preserved for single-target removal.


For bounded verification of uncertain references, use `--precise-selective` or
MCP `precise_selective: true`; see [selective verification](precise-selective.md).
Inspect `metadata.precision`; selective failures keep heuristics and return partial.


Selective CLI flags are supported by sync, graph, serve, explain, impact and path.
Report/digest/search consume stored evidence; sync first when verification is
needed. The 30-second selective budget is configured by `[precise]
selective_budget_secs`, whereas `--precise-timeout` bounds an individual request.
`--precise-full` forces the selected mode; it does not mean a full source reparse.
With no mode configured, plain sync starts no LSP. Project `[precise] selective=true`
opts CLI sync/graph/serve into selective mode; an explicit --precise overrides it.
