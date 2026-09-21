---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# Coding Conventions

**Analysis Date:** 2026-09-21

## Naming Patterns

**Files:** snake_case Rust modules (`src/analysis/rust/macros.rs`, `src/cache/mutations.rs`). Directory modules use `mod.rs` (`src/commands/digest/mod.rs`). Language analyzers live in `src/analysis/<lang>.rs`.

**Functions:** snake_case (`ensure_extension`, `build_graph`). Visibility-limited helpers use `pub(crate)`.

**Types:** PascalCase (`CacheDb`, `SymbolRow`, `DigestArgs`); CLI arg structs end in `Args` in `src/cli.rs`; DB row structs end in `Row` in `src/cache/models.rs`.

**Constants:** SCREAMING_SNAKE (`BIN` in tests).

## Code Style

**Formatting:** Default rustfmt (edition 2024); no `rustfmt.toml`. Run `cargo fmt`.
**Linting:** No `clippy.toml`; use default `cargo clippy`.

## Import Organization

1. `anyhow` and other external crates
2. `std::...`
3. `crate::...`

Modules declare `pub mod x;` then `pub use x::*;` re-exports at the top (`src/commands/digest/mod.rs`). Use `crate::` absolute paths, not path aliases.

## Error Handling

- Use `anyhow::Result` everywhere; no `thiserror`.
- Add context with `.with_context(|| format!("creating {}", dir.display()))?` (`src/cache/mod.rs`).
- Validation failures use `anyhow::bail!("unknown --scope `{other}` (expected file, symbol, or both)")` (`src/commands/graph.rs`), giving the offending value and the valid options.

## Logging

Plain `eprintln!` for progress/diagnostics to stderr; stdout is reserved for output payloads. No logging framework.

## Comments

`//!` module docs explaining why (tests and analyzers), `///` on helpers, inline `//` for rationale. Doc comments describe the bug/behavior being pinned.

## Function Design

Small helpers taking `&Path`/`&str`; return `Result<T>` for I/O. Command entry points take the clap `*Args` struct.

## Module Design

Commands in `src/commands/<name>/` (`mod.rs`, plus `render.rs`, `handlers.rs`, `tools.rs`); analyzers in `src/analysis/`; persistence in `src/cache/`; graph in `src/graph/`. Glob re-exports via `pub use`.

---

*Convention analysis: 2026-09-21*
