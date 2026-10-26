---
name: code-rcl
description: Inspect code structure, symbol relationships, and change impact with code-rcl CLI or MCP. Use for architecture and dependency questions; use text search for literal text and unsupported syntax.
metadata:
  version: "0.10.0"
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
- `precise: true` / `--precise` asks installed language servers for definitions.
  Missing servers, timeout, and incomplete indexing fail explicitly. Only use
  `allow_heuristic_fallback: true` / `--allow-heuristic-fallback` when partial
  results are acceptable; inspect warnings and resolution mode afterward.
- Queries may reuse valid cached LSP edges. Editing any analyzed file invalidates
  precise results for the entire project.
- `.code-rcl/REPORT.md` is a saved snapshot; run `report` when freshness matters.

## CLI versus MCP

Prefer configured MCP tools when available. MCP results always contain the
versioned structured envelope; do not pass the removed `json` parameter.
Use an explicit absolute `project` when the server has no configured root.

For CLI, resolve the binary on PATH, then check the per-user installation.
Check `--version` and `--help` for compatibility with this 0.10.0 skill.

When developing **code-rcl itself**, build and test the current checkout with
Cargo. Use that build to verify changes; an installed release can differ from
working-tree source. In other projects, use the installed tool.

## Conditional references

- [Embeddings](references/embeddings.md): preparation, freshness, and retrieval modes.
- [CLI](references/cli.md): flags, JSON envelope, common examples.
- [MCP](references/mcp.md): tool parameters, budgets, errors, cancellation.
- [Installation](references/installation.md): binary discovery and explicit setup.

Use generated documentation and help for exact parameters. Do not promise fixed
latency, token savings, or complete call-graph accuracy without measurements.
