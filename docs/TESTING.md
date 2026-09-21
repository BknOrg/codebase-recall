<!-- generated-by: gsd-doc-writer -->
# Testing

## Test framework and setup

The project is a Rust crate (`codebase-recall`, edition 2024, binary `code-rcl`) and uses Cargo's built-in test harness (`#[test]`). No extra test dependencies are declared in `Cargo.toml`. You only need a stable Rust toolchain:

```bash
cargo build
```

There are two layers of tests:

- **Unit tests** live inline in `src/` inside `#[cfg(test)]` modules (for example `src/analysis/go.rs`, `src/analysis/python/python.rs`, `src/commands/impact/traversal.rs`, `src/graph/community.rs`).
- **Integration tests** live in `tests/*.rs`. They run the compiled `code-rcl` binary (via `env!("CARGO_BIN_EXE_code-rcl")`) against sample projects under `tests/fixtures/`.

| Integration test file | Focus |
|-----------------------|-------|
| `tests/digest_cli.rs` | `digest` command output |
| `tests/graph_cli.rs` | `graph` command |
| `tests/impact_cli.rs` | `impact` command |
| `tests/report_cli.rs` | report generation |
| `tests/serve_cli.rs` | `serve` command |
| `tests/setup_cli.rs` | `init`/setup |
| `tests/sync_cli.rs` | index sync |
| `tests/config_keys.rs` | config key extraction |
| `tests/macro_bodies.rs` | Rust macro body analysis |
| `tests/type_edges.rs` | type edges |
| `tests/resolve_accuracy.rs` | call-edge resolution accuracy against `expected-edges.json` |

## Running tests

Full suite (unit and integration):

```bash
cargo test
```

Unit tests only:

```bash
cargo test --bin code-rcl
```

A single integration test file:

```bash
cargo test --test digest_cli
```

A single test by name:

```bash
cargo test digest_generates_markdown_outline
```

Show printed output (useful for skipped-test messages):

```bash
cargo test -- --nocapture
```

### Optional language-server tests

The precise-resolution tests in `tests/resolve_accuracy.rs` are opt-in. They are skipped (with a printed message) unless the environment variable `CODE_RCL_TEST_PRECISE` is set, and they require the relevant language servers to be installed and on `PATH`:

```bash
CODE_RCL_TEST_PRECISE=1 cargo test --test resolve_accuracy -- --nocapture
```

On PowerShell:

```powershell
$env:CODE_RCL_TEST_PRECISE = "1"; cargo test --test resolve_accuracy -- --nocapture
```

## Writing new tests

- **Unit tests:** add a `#[cfg(test)] mod tests { ... }` block at the bottom of the source file you are changing.
- **Integration tests:** add a `tests/<name>.rs` file. The existing files follow this pattern:
  - `const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");` to locate the built binary.
  - A `fixture(name)` helper resolving `tests/fixtures/<name>` from `CARGO_MANIFEST_DIR`.
  - A `workdir(name, tag)` helper that copies a fixture into `CARGO_TARGET_TMPDIR` so tests never modify the checked-in fixtures (and so the index/cache is created in a scratch copy).
  - Run the binary with `std::process::Command`, then assert on exit status and stdout.
- **Fixtures:** each sample project is a directory in `tests/fixtures/` (for example `rust_app`, `py_app`, `java_app`, `kotlin_app`, `resolve_app`). Use a unique `tag` per test so parallel tests do not share a working directory.
- **Accuracy fixtures:** fixtures used by `resolve_accuracy.rs` (such as `resolve_app`, `local_shadow_app`, `python_precise_app`, `rust_precise_app`) include an `expected-edges.json` listing required call edges and forbidden ones. Extend that file when adding cases.

## Coverage requirements

No coverage threshold configured. There is no coverage tool configuration in the repository.

## CI integration

The only workflow is `.github/workflows/release.yml` (name: Release). It triggers on pushed tags matching `v*` and builds release binaries for Linux (musl), Windows (MSVC) and macOS (x86_64 and aarch64) with `cargo build --release`. It does not run `cargo test`, so run the suite locally before tagging a release.
