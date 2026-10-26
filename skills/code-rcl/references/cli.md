# CLI reference - 0.10.0

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
code-rcl sync --precise
```

JSON results use `schema_version`, `status`, `data`, and `metadata`. Former
root-level arrays/fields are now inside `data`; graph data retains `version: 2`.
Impact depth is 1-8, path 1-20, focused dump/graph 1-5 for MCP.
`--allow-heuristic-fallback` explicitly permits partial precise results.
`--no-sync` skips refresh and marks freshness unverified.
The cache is `.code-rcl/cache.bkndb`; schemas 10/11 migrate additively to 12,
preserving analysis and valid vectors. Older incompatible layouts are rebuilt.

Search supports `--match-mode exact|prefix|substring` (MCP: `match_mode`); exact and prefix use the normalized-name index. Default is substring.

`--lexical` forces lexical lookup. Lexical result groups share one output limit;
`total_matches` counts eligible items before that limit, with `match_counts`
per group. Automatic task retrieval remains release-gated; see embeddings.md.

For semantic/hybrid retrieval and explicit model/index setup in 0.10.0, read
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
