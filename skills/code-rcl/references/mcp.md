# MCP reference - 0.9.1

Transport: newline-delimited JSON-RPC over stdio, protocol 2025-06-18.
Initialize before tools/list or tools/call, then notify notifications/initialized.
Read tools/list for schemas generated from the server input types.

All tools accept project, no_sync, and max_output_kb (default 256, range 1-2048).
Do not combine no_sync with sync or precise. Unknown parameters and invalid
values are errors; the old json flag has been removed.

- code_rcl_report: architecture overview.
- code_rcl_digest: path, all, with_docs, doc_lines.
- code_rcl_search: query, retrieval (auto/lexical/semantic/hybrid), match_mode,
  kind, exported_only, strings, limit (1-100). The default stays lexical until
  automatic feature retrieval passes its accuracy and latency gates.
- code_rcl_impact: symbol OR diff=true; depth 1-8; direction both/reverse/forward;
  kinds; precise OR precise_selective; allow_heuristic_fallback.
- code_rcl_path: from, to, max_depth 1-20, direction forward/reverse/any; kinds;
  precise OR precise_selective; allow_heuristic_fallback.
- code_rcl_explain: symbol; precise OR precise_selective; allow_heuristic_fallback.
- code_rcl_dump: target, depth 1-5, max_size_kb per file (1-10240).
- code_rcl_sync: languages filters both AST and LSP; precise OR precise_selective;
  embeddings; full forces selected precise queries and/or embedding rebuilds;
  at least one mode is required (including sync/graph configuration defaults).
  allow_heuristic_fallback is needed only for strict full-mode fallback.
- code_rcl_graph: scope file/symbol/both; focus; depth 1-5; max_nodes (default 2000);
  kinds (default imports/calls/contains); min_confidence (0-1, default 0);
  precise OR precise_selective; allow_heuristic_fallback. Edge kinds are calls/imports/references/contains/
  inherits/implements/overrides; impact/path accept the same kinds.

Results appear in structuredContent and as the same serialized JSON text.
Inspect schema_version=2, status, data, metadata.project, cache_generation,
freshness, resolution_mode, warnings, truncated, and omitted_count.
Graphs use data.version=3. Inspect resolution_summary and per-relation resolution
evidence, ambiguity and alternatives. Guesses are capped at 0.35; min_confidence=0.5
hides them. Use returned declaration IDs or qualified names for overloads; IDs
contain byte ranges, not lines. Unknown semantic facts do not mean false facts.
A response budget may omit whole items/files; narrow the query or raise the
budget when metadata reports truncation. Diff omits isolated targets and limits
reports to the ten most affected targets, with omitted counts recorded.

notifications/cancelled accepts requestId. Ping remains available during analysis.
Analysis is serialized; cancellation is cooperative at analysis checkpoints.

Search supports `--match-mode exact|prefix|substring` (MCP: `match_mode`); exact and prefix use the normalized-name index. Default is substring.

For semantic/hybrid retrieval and explicit model/index setup in 0.9.1, read
[embeddings.md](embeddings.md). Source and embedding freshness are separate.

Embedding-only rebuild: call code_rcl_sync with {"embeddings":true,"full":true}.
This requires a prepared local model, not a precise/LSP backend.

Search/report accept languages, paths, exclude_paths and categories arrays.
Category globs must be configured by the project; default scope remains all code.
Put a progressToken in tools/call params._meta (not arguments) for progress
notifications. A heartbeat signals liveness, not completed inference.
Sync results include embedding statistics and final operation metadata.
Inspect metadata.reasons for candidate limits, output budgets and partial coverage.
Explain retains symbol identity/counts before trimming relations. If the minimum
summary does not fit, raise max_output_kb; an explicit budget error is not absence.
Semantic detail may be trimmed before identity/counts; inspect trimmed_details.
Cache upgrades to schema 13 invalidate derived analysis/vectors while preserving
source snapshots. no_sync rejects stale semantic generations; use normal sync.
Read [symbol semantics](symbols.md) for output interpretation and migration.

Embedding sync statistics distinguish logical new chunks from unique native
inferences and deduplicated chunks. Both MCP and CLI honor project
`batching=file|length`; changing batching requires no rebuild. `full` remains
the ordinary chunk profile. Compact summaries are a gated harness experiment
with a distinct pipeline and source provenance; do not enable them implicitly.


For bounded verification of uncertain references, use `--precise-selective` or
MCP `precise_selective: true`; see [selective verification](precise-selective.md).
Inspect `metadata.precision`; selective failures keep heuristics and return partial.


## Resolution requests

Only sync, graph, explain, impact and path accept `precise_selective`. Report,
digest and search consume cached evidence and expose coverage; do not pass that
argument to them. Verify supported properties in the running `tools/list`.

Call `code_rcl_sync` with these **arguments** (not a JSON-RPC envelope):

```json
{"project":"/absolute/project","precise_selective":true}
```

Then request the architecture or a focused symbol without repeating a force
operation. For a focused verified query, `code_rcl_explain` accepts
`{"project":"/absolute/project","symbol":"QualifiedName","precise_selective":true}`.
The symbol/path/graph display scope does not bound the LSP work; reference selection
is independent of node caps and display filters. Source `languages` on sync can
restrict analyzed languages explicitly.

To force checks, use sync arguments `precise_selective: true, full: true`;
`precise_full` is a CLI flag name, not an MCP parameter. Selective mode uses the
project's `selective_budget_secs` (30 by default). Missing LSPs return partial,
not a successful full verification. `metadata.precision.languages` describes
coverage; `metadata.precision.complete` is relative to the requested policy.
No new LSP/model/build is installed or started as a setup side effect.

Config `[precise] selective=true` supplies omitted mode arguments to MCP sync and
graph, unless no_sync is requested. An explicit mode field overrides those defaults;
`precise:false` disables configured mode selection for that request. Explain,
impact and path need an explicit mode argument to request LSP verification.
See [selective verification](precise-selective.md) for interpretation and recovery.
