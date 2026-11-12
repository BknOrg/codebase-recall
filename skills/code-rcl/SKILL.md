---
name: code-rcl
description: Inspect code structure, symbol relationships, and change impact with code-rcl CLI or MCP. Use for architecture and dependency questions; use text search for literal text and unsupported syntax.
metadata:
  version: "0.9.1"
  capabilities: "graph-3 envelope-2 selective-1"
---

# code-rcl

Use code-rcl to narrow what you need to inspect. Its graph is evidence from
parsers and optional language servers, not proof that every dependency is known.

## Choose an entry point

- Architecture: `report` / `code_rcl_report`, then `digest` for declarations.
- Identifier lookup: lexical `search` / `code_rcl_search` (default).
- Behavioral descriptions: `search --semantic` / search with `retrieval: semantic`.
- Mixed identifier and intent: `search --hybrid` / `retrieval: hybrid`.
- Semantic/hybrid require an explicitly prepared model. Normal searches refresh
  pending embeddings in scope; explicit initial indexing can schedule that cost.
- Batching can reduce padding; it does not reduce source coverage. Keep the full
  profile for ordinary use; compact container summaries remain an experiment.
- Understand a symbol: `explain` / `code_rcl_explain`, then read relevant source.
- Review callers/callees: `impact SYMBOL` / `code_rcl_impact` with `symbol`.
- Review tracked changes: `impact --diff` / `code_rcl_impact` with `diff: true`.
- Connect two symbols: `path FROM TO` / `code_rcl_path`.
- Gather source context: targeted `dump` / `code_rcl_dump`.
- Inspect graph data: `graph` / `code_rcl_graph`.

Prefer a focused query over dumping a complete repository. Use text search when
looking for comments, unsupported syntax, or evidence absent from the graph.
Do not run every tool in sequence for a small task.

## Verify before drawing conclusions

- Check `status`, `metadata.warnings`, `freshness`, and `truncated`.
- For semantic/hybrid, also check `metadata.embeddings`: its freshness and
  pending/excluded files differ from source freshness. Partial indexes exclude invalid
  chunks; standalone source sync does not rebuild embeddings, while normal
  semantic/hybrid search refreshes pending in-scope files before retrieval.
- For slow sync, inspect phase, heartbeat and completed work. Use status --files
  for parser blocks; cache-busy status uses an unverified saved snapshot.
- Resume interrupted indexing with ordinary sync; --full deliberately rebuilds.
- Narrow language/path/category scope explicitly; do not silently discard tests.
- Similarity and RRF scores rank candidates; they do not establish dependency
  edges or probabilities. Verify relevant source and graph relationships.
- Model download is explicit. Report missing setup; never download on search.
  Normal semantic searches can index pending files; use --no-sync only when the
  user accepts cached, unverified state.
- `--no-sync` / `no_sync: true` uses an unverified cache. Normal sync hashes
  source content; only changed content is reparsed.
- An empty result can mean missing parser coverage, ambiguous symbols, or an
  incomplete graph. Inspect source before calling a change safe.
- Diff compares HEAD with tracked working-tree content (staged and unstaged).
  It includes deleted symbols but excludes untracked files. Baseline edges use
  heuristics; they are not compiler-verified historical results.
- Heuristic confidence is a ranking signal, not a calibrated probability.
- Read `resolution_summary` and per-relation `resolution.ambiguous`, reasons,
  and alternatives. A selected guess (confidence at most 0.35) is not a verified
  dependency; alternatives do not add graph edges. Inspect source or use precise
  resolution when the distinction matters.
- Distinguish semantic kinds and owners: constructors, fields, properties and
  enum members are separate declarations. Unknown facts remain unknown. Use a
  qualified name or returned declaration ID for overloads; IDs contain byte
  ranges, not line numbers. Read `lines`, `path`, and semantic name ranges.
- Choose a resolution mode for the task: ordinary queries use heuristics and
  valid cached LSP answers; selective verification is an opt-in speed/accuracy
  tradeoff, while full precise verification is strict. Do not start LSP for every
  lookup. Read [selective verification](references/precise-selective.md) when
  extra verification is requested or necessary for the conclusions.
- `precise_selective: true` / `--precise-selective` checks uncertain/risky reference
  sites with a shared 30-second LSP budget by default. Missing servers and errors
  keep remaining heuristics and return partial coverage, without installing tools.
- `precise: true` / `--precise` checks all pending reference sites. Failures require
  explicit `allow_heuristic_fallback: true` / `--allow-heuristic-fallback` to return
  partial output. Never combine full and selective mode.
- Inspect `metadata.precision.languages` for candidates, cache_hits, queried,
  verified, unresolved, skipped, failed and pending. Selective `complete: true`
  means its policy completed; it does not verify every reference. Interpret
  `resolution_mode` as actual evidence (`heuristic`, `mixed`, `precise_cached`).
- Recheck with CLI `--precise-full` alongside the chosen mode, or MCP sync
  `full: true` alongside `precise_selective: true` or `precise: true`. A force
  operation still respects the selective budget. Ordinary retries reuse valid
  answers; transient failures have a 60-second cooldown. Do not retry in a loop.
- Queries may reuse valid cached LSP edges. Editing any analyzed file invalidates
  precise results for the entire project.
- `.code-rcl/REPORT.md` is a saved snapshot; run `report` when freshness matters.

## CLI versus MCP

Prefer configured MCP tools when available. MCP results always contain the
structured envelope (`schema_version=2`, graph `data.version=3`); do not pass the
removed `json` parameter. Check the running server's contract before interpreting
cached responses from another build.
Use an explicit absolute `project` when the server has no configured root.

For CLI, resolve the binary on PATH, then check the per-user installation.
Check `--version` and `sync --help`, or MCP `tools/list`. The package version
alone does not establish feature support: selective mode requires
`--precise-selective` / the `precise_selective` input property. If absent, the
running binary/server is older than this skill; use the current build and
refresh the integration rather than inventing unsupported arguments.

When developing **code-rcl itself**, build and test the current checkout with
Cargo. Use that build to verify changes; an installed release can differ from
working-tree source. In other projects, use the installed tool.

## Conditional references

- [Selective verification](references/precise-selective.md): mode choice, exact
  tool support, coverage, backend requirements, retry and configuration examples.

- [C/C++](references/c-family.md): clangd, build context, header language, and limits.
- [Embeddings](references/embeddings.md): preparation, freshness, and retrieval modes.
- [CLI](references/cli.md): flags, JSON envelope, common examples.
- [MCP](references/mcp.md): tool parameters, budgets, errors, cancellation.
- [Installation](references/installation.md): binary discovery and explicit setup.
- [Symbol semantics](references/symbols.md): ambiguity, declaration identity,
  structural traversal, cache migration, and output details.

Use generated documentation and help for exact parameters. Do not promise fixed
latency, token savings, or complete call-graph accuracy without measurements.
