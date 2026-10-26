# Migrating to code-rcl 0.9.0

This release changes CLI JSON, MCP inputs/results, precise failure handling,
and the local viewer API. Rebuild integrations before upgrading production use.

## Results

CLI JSON and MCP structured results use `schema_version: 1`, `status`, `data`,
and `metadata`. Read former root-level fields/arrays through `data`.
Graph data still has its internal `version: 2` format.
Check warnings, freshness, resolution_mode, truncated, and omitted_count.
MCP provides the same envelope in structuredContent and serialized text.
MCP no longer accepts `json`; unknown or mistyped arguments are rejected.
max_output_kb defaults to 256, with a maximum of 2048, including both serialized
representations. Limits drop complete items/files, never split source snippets.

## Precise and cache

Precise requests fail if a required language server is missing, fails, or does
not finish indexing. `--allow-heuristic-fallback` (MCP:
`allow_heuristic_fallback: true`) explicitly permits partial heuristic results.
An analysis change invalidates the project's cached precise pass.
Sync hashes eligible source files every time; only changed content is parsed.
The schema upgrade rebuilds derived cache data. Configuration and reports are
preserved. Storage I/O errors are reported rather than deleting the cache.

## Diff

Impact diff compares HEAD to tracked working-tree contents, including staged
and unstaged changes and deleted definitions. `change_kind` identifies added,
modified, or deleted targets. Renames are deletion/addition pairs; untracked
files are excluded. The baseline uses heuristic AST resolution without running
historical build scripts. Reports can conservatively include symbols touched
on either side of a change; this is structural impact, not semantic equivalence.

## MCP and setup

The protocol is 2025-06-18. Initialize and send notifications/initialized before
using tools. Invalid requests/arguments produce JSON-RPC errors; execution
failures produce isError=true. Analysis is serialized; ping and cooperative
cancellation remain available. Cancelled requests do not send result messages.
Run workspace setup again to pin its absolute project root and install the
complete skill bundle, including references. Servers without --project require
an explicit project per tool call. Global setup is opt-in.

## Viewer

Mutation endpoints /dump, /impact, /quit require POST and a session token in
X-Code-Rcl-Token. The viewer embeds the token in its page. API reads require
the token too; EventSource supplies it as a query parameter. Host/Origin checks
and filesystem capability confinement prevent crossing the project boundary.
Existing output files return HTTP 409; choose a different name. Requests that
omit a filename receive a unique filename.

## Development

Use the current checkout's Cargo build to test code-rcl itself. The installed
binary may differ from source. Git is required for diff tests; mock LSP tests
run without installed language servers. Real-LSP tests remain opt-in.
Run cargo fmt, cargo clippy --all-targets -- -D warnings, cargo test --all-targets,
and the no-default-features checks before release.
