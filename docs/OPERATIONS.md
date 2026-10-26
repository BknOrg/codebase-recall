# Long-running embedding operations

Run `code-rcl embeddings setup` explicitly before indexing. Normal search never
downloads a model or silently embeds pending source.

```sh
code-rcl embeddings sync
code-rcl embeddings sync --json --progress plain
code-rcl embeddings status --json
code-rcl embeddings status --files --json
code-rcl embeddings status --path bindings/generated/api.kt --json
```

## Progress and cancellation

`--progress auto` is the default: a live summary on an interactive stderr,
rate-limited lines otherwise. `plain` always uses lines; `off` suppresses the
renderer. stdout with `--json` contains one final envelope, including failures
after the operation starts. Progress never shares stdout with CLI JSON or MCP.

The summary distinguishes opening the cache, source verification, model checksum
verification, runtime loading, chunking, inference, commit and finalization.
It reports the active file, files processed, chunks newly inferred/reused and
committed, batch, elapsed time and time since the last completion. Chunk totals
are unknown before chunking. No ETA or invented global percentage is shown.

A one-second heartbeat means the process is alive. It does **not** mean an
inference batch completed. Native model loading and inference can take several
seconds without a completed batch. Check phase, `cancel_requested`, and elapsed
time before concluding that a process is stuck.

Press Ctrl+C once to request cancellation. The current native batch/load must
return before cooperative cancellation is checked. Completed file transactions
remain valid; the current uncommitted file is discarded. A second Ctrl+C forces
exit. Rerun ordinary `embeddings sync` to resume; do not use `--full` just to
recover, because full deliberately clears the derived embedding index.

## Status while indexing

`.code-rcl/embedding-operation.json` is an atomic, per-project operation snapshot.
Its companion OS lock determines whether the owner is still active. A saved PID
or recent heartbeat alone is not proof that a process is alive. If a process is
killed before finalizing, status calls its saved running snapshot `interrupted`.

`embeddings status` first reads the snapshot. When the main cache is busy, it
returns available saved coverage plus live operation information, marks source
freshness unverified and `database_busy:true`. Cached coverage is not a fresh
database query. Per-file diagnostics may be unavailable until the owner releases
the cache. A database lock without a snapshot remains an explicit busy error.
Never delete the database or lock files to bypass an active process.

File coverage states are `ready`, `pending_index`, `blocked_parser`, and
`excluded_configuration`. `pending_files` includes blocked files; repeated sync
alone does not repair a parser/grammar failure. `--files` includes source hash,
cached diagnostics and recovery instructions. `--path` inspects a requested
project-relative path and reports a matching Git ignore rule without scanning
ignored trees. It requires a Git project for ignore-rule inspection.

Analyzer revisions trigger reparsing even when source hashes are unchanged.
Diagnostics survive warm-cache reads and are replaced/deleted with source.
Kotlin block-comment normalization handles a reproduced grammar ambiguity while
preserving original bytes/lines and still rejecting broken syntax. It is an AST
parse workaround, not text extraction or compiler-grade LSP resolution.

## MCP and limited responses

The nine tool names are unchanged. Put a string/number `progressToken` in
`tools/call` **params._meta**, outside tool arguments, to receive standard
`notifications/progress`. Notifications carry elapsed progress and a message
containing the operation snapshot. With no token, no notifications are sent.
Ping/cancellation remain available during native inference.

Sync results retain source statistics and include embedding statistics; operation
metadata includes the final state and phase timings. Inspect `metadata.reasons`
to distinguish parser errors, embedding coverage, unverified source, candidate
caps, result limits and response budgets.

MCP budgets include both structured and text copies. Explain trims relations and
documentation before symbol identity and retains original relation counts.
Search shortens snippets before dropping candidates. Dump omits whole files;
graph output keeps node-edge consistency. If a minimum identity/summary cannot
fit, an explicit budget error replaces misleading empty success. Increase
`max_output_kb` or narrow the query; similarity is not dependency evidence.

## Timing and evaluation

`metadata.timings_ms` exposes measured source read/hash, parsing, model verify/load,
tokenization, inference, row decoding, commits, ranking, fusion and serialization
or budgeting where applicable. Phase timings can be nested: do not add them as
independent wall-clock costs. Operation timing and external process/MCP latency
are separate measurements. Download is excluded from model tests and retrieval
benchmarks that use `--model-dir`.

See [retrieval evaluation](RETRIEVAL-EVALUATION.md) for the pinned dataset,
offline experiments and local profiling observations. No universal latency or
accuracy guarantee follows from these measurements.

A no-op incremental sync still verifies the six pinned model files but skips
native model initialization when there are no files requiring inference.
`unchanged_files` is separate from chunk reuse within changed files. Missing or
corrupt models still fail explicitly. Saved failure snapshots include the error.
Selecting 256 dimensions reduces vector storage/scoring work; it does not avoid
running the 768-output transformer. Review language/ignore scope explicitly for
large generated sources rather than using --full as a speed/recovery option.

Combined MCP syncs also label the optional LSP pass `precise_resolution`;
completion of embedding work does not imply that a requested LSP pass succeeded.
