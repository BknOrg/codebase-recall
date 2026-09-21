---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# Codebase Concerns

**Analysis Date:** 2026-09-21

## Tech Debt

**Uncommitted large in-flight change set:**

- Issue: Many staged/modified files (`src/analysis/rust/macros.rs`, `src/analysis/rust/types.rs`, `src/analysis/toml.rs`, `src/graph/resolve/refs.rs`, `src/cache/schema.rs`) are not yet committed.
- Impact: Schema changes in `src/cache/schema.rs` may require cache invalidation/migration for existing users.
- Fix approach: Verify schema version bump and cache rebuild path before release.

**Per-language analyzers are large and duplicated in shape:**

- Files: `src/analysis/javascript.rs` (643 lines), `src/analysis/go.rs` (559), `src/analysis/java.rs`
- Impact: Fixes to reference/type-edge resolution must be replicated per language; Rust has the richest support (macros, types) while others lag.
- Fix approach: Extract shared extraction helpers into `src/analysis/mod.rs`.

**Large command modules:**

- Files: `src/commands/setup_integrations.rs` (553), `src/cli.rs` (546), `src/commands/report.rs` (522), `src/server/mod.rs` (522), `src/precise/client.rs` (534)

## Known Bugs (from `CODE_RCL_ANALYSIS_ISSUES_REPORT.md`)

**Async macro call chains lost:**

- Symptoms: Functions called inside `futures::try_join!` report 0 callers.
- Files: `src/analysis/rust/macros.rs`, `src/analysis/rust/calls.rs`
- Note: `tests/macro_bodies.rs` covers only part of this.

**`path` command returns "No path found" for valid chains:**

- Files: `src/commands/path.rs`, `src/graph/resolve/`
- Trigger: e.g. `main` to a command handler through macro or trait dispatch.

**No runtime/state semantics:** file locks and concurrency are not modeled.

## Security Considerations

**MCP server and web server expose the local codebase:**

- Files: `src/commands/mcp/handlers.rs`, `src/server/mod.rs`
- Risk: Path or symbol inputs handled from external clients; the server binding address and path validation should be confirmed.
- Recommendations: Bind to loopback only; canonicalize and restrict paths to the project root.

**Integration setup writes to user config:**

- Files: `src/commands/setup_integrations.rs`
- Risk: Modifies agent config files outside the repo. Tested by `tests/setup_cli.rs`.

## Performance Bottlenecks

**Full-graph traversal for impact/path:**

- Files: `src/commands/impact.rs`, `src/commands/path.rs`, `src/graph/`
- Cause: Reverse traversals on big repos (report tested against a 14-file blast radius). No documented limits.

## Fragile Areas

**Name-based reference resolution:**

- Files: `src/graph/resolve/mod.rs`, `src/graph/resolve/refs.rs`
- Why fragile: Heuristic resolution of names produces false or missing edges; guarded by `tests/resolve_accuracy.rs` and `tests/type_edges.rs`.
- Safe modification: Run all fixtures under `tests/fixtures/` after any change.

**Panics via unwrap/expect:**

- Around 130 `unwrap()`/`expect()` occurrences in `src/`, mostly in inline tests; audit non-test uses in the analyzers.

**Precise (external) client:**

- Files: `src/precise/client.rs`
- Depends on external language servers or tools; failures should degrade gracefully.

## Scaling Limits

Not measured. SQLite cache in `src/cache/` is single-project and local.

## Dependencies at Risk

Not audited; no lockfile audit tooling is detected. CI lives in `.github/workflows`.

## Missing Critical Features

- Semantic modeling of runtime behavior (locks, concurrency).
- Type-edge and macro-body support beyond Rust is not detected.

## Test Coverage Gaps

**Non-Rust analyzers:**

- Files: `src/analysis/go.rs`, `src/analysis/javascript.rs`, `src/analysis/java.rs`
- Only inline unit tests; no integration fixtures comparable to `tests/fixtures/type_edges_app`.
- Priority: Medium

**TOML config-key analysis:**

- Files: `src/analysis/toml.rs`; covered by a single `tests/config_keys.rs`.
- Priority: Low

---

*Concerns audit: 2026-09-21*
