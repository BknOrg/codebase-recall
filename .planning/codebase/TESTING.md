---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# Testing Patterns

**Analysis Date:** 2026-09-21

## Test Framework

**Runner:** Built-in Rust `cargo test` (no external framework, no dev-dependencies listed in `Cargo.toml`).
**Assertion Library:** std `assert!`, `assert_eq!`; `serde_json::Value` for JSON output inspection.

**Run Commands:**

```bash
cargo test                     # All unit + integration tests
cargo test --test type_edges   # One integration file
cargo test flattens_use        # By name
```

No coverage tooling configured.

## Test File Organization

**Unit tests:** inline `#[cfg(test)] mod tests` at the bottom of source files (`src/analysis/rust/mod.rs`, `src/analysis/go.rs`, `java.rs`, `kotlin.rs`, `javascript.rs`, `sfc.rs`, `toml.rs`, `python/python.rs`, `src/cache/utils.rs`, `src/commands/digest/extractor.rs`).

**Integration tests:** `tests/*.rs`, one per command or feature: `digest_cli.rs`, `graph_cli.rs`, `impact_cli.rs`, `report_cli.rs`, `serve_cli.rs`, `setup_cli.rs`, `sync_cli.rs`, `resolve_accuracy.rs`, `type_edges.rs`, `macro_bodies.rs`, `config_keys.rs`.

**Fixtures:** `tests/fixtures/<name>_app/` (e.g. `type_edges_app/`, `macro_body_app/`, `config_keys_app/`).

## Test Structure

Unit tests parse a source string and assert on the result:

```rust
let p = parse(src);
let spec = |s: &str| p.imports.iter().find(|i| i.raw_specifier == s);
assert!(spec("std::collections::HashMap").is_some());
```

Integration tests run the real binary and copy fixtures into a per-test temp dir:

```rust
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");
fn workdir(tag: &str) -> PathBuf { /* copy tests/fixtures/x_app into CARGO_TARGET_TMPDIR/x__{tag} */ }
let result = Command::new(BIN).args(["graph", "--project"]).arg(work)...output()?;
assert!(result.status.success(), "graph failed: {}", String::from_utf8_lossy(&result.stderr));
```

Each file starts with a `//!` doc explaining the regression being pinned. Include stderr in failure messages.

## Mocking

None. No mocking framework; tests use real SQLite cache, real tree-sitter parsing, and the compiled binary against fixture projects.

## Fixtures and Factories

Fixture apps are small multi-file projects under `tests/fixtures/`. Helper `workdir(tag)` (duplicated per test file) copies to a clean directory under `CARGO_TARGET_TMPDIR` using a unique tag per test to allow parallelism.

## Coverage

None enforced. CI (`.github/workflows/release.yml`) only builds release targets; it does not run tests.

## Test Types

- Unit: language analyzers, extraction, utilities.
- Integration/CLI: end-to-end via the binary, checking JSON output (`--format json`).
- E2E/browser: not used (`serve_cli.rs` tests the server through the CLI).

## Common Patterns

Parse JSON output: `serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap()`, then collect into `HashSet` for membership assertions. Use `.unwrap()`/`.expect("failed to run ...")` freely in tests.

---

*Testing analysis: 2026-09-21*
