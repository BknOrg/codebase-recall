# MCP reference - 0.10.0

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
  kinds; precise; allow_heuristic_fallback.
- code_rcl_path: from, to, max_depth 1-20, direction forward/reverse/any; kinds;
  precise; allow_heuristic_fallback.
- code_rcl_explain: symbol; precise; allow_heuristic_fallback.
- code_rcl_dump: target, depth 1-5, max_size_kb per file (1-10240).
- code_rcl_sync: languages filters both AST and LSP; precise; embeddings; full requires precise or embeddings;
  full rebuilds only selected backends; allow_heuristic_fallback.
- code_rcl_graph: scope file/symbol/both; focus; depth 1-5; max_nodes (default 2000);
  precise; allow_heuristic_fallback.

Results appear in structuredContent and as the same serialized JSON text.
Inspect schema_version=1, status, data, metadata.project, cache_generation,
freshness, resolution_mode, warnings, truncated, and omitted_count.
A response budget may omit whole items/files; narrow the query or raise the
budget when metadata reports truncation. Diff omits isolated targets and limits
reports to the ten most affected targets, with omitted counts recorded.

notifications/cancelled accepts requestId. Ping remains available during analysis.
Analysis is serialized; cancellation is cooperative at analysis checkpoints.

Search supports `--match-mode exact|prefix|substring` (MCP: `match_mode`); exact and prefix use the normalized-name index. Default is substring.

For semantic/hybrid retrieval and explicit model/index setup in 0.10.0, read
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

Embedding sync statistics distinguish logical new chunks from unique native
inferences and deduplicated chunks. Both MCP and CLI honor project
`batching=file|length`; changing batching requires no rebuild. `full` remains
the ordinary chunk profile. Compact summaries are a gated harness experiment
with a distinct pipeline and source provenance; do not enable them implicitly.
