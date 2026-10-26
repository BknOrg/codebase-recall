# Testing

## Test framework and setup

The project is a Rust crate (`codebase-recall`, edition 2024, binary `code-rcl`) and uses Cargo's built-in test harness (`#[test]`). Fixture isolation uses temporary directories. The mandatory mock-LSP driver runs as a native subprocess without installed language servers. Use a current stable Rust toolchain, a C compiler, and Git:

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
| `tests/sync_cli.rs` | content hashes, metadata, generation invalidation, schema rebuild and cache errors |
| `tests/mcp_cli.rs` | subprocess protocol, validation, budgets, lifecycle and cancellation |
| `tests/search_cli.rs` | indexed modes and versioned JSON |
| `tests/support/mock_lsp.rs` | native LSP mock: success, initialization/request/indexing errors, fallback and cleanup |
| `tests/config_keys.rs` | config key extraction |
| `tests/macro_bodies.rs` | Rust macro body analysis |
| `tests/type_edges.rs` | type edges |
| `tests/resolve_accuracy.rs` | call-edge resolution accuracy against `expected-edges.json` |

## Running tests

Full suite (unit and integration):

```bash
cargo test --all-targets
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

The precise-resolution tests in `tests/resolve_accuracy.rs` are opt-in. They are marked ignored and additionally require CODE_RCL_TEST_PRECISE=1, and they require the relevant language servers to be installed and on `PATH`:

```bash
CODE_RCL_TEST_PRECISE=1 cargo test --test resolve_accuracy -- --ignored --nocapture
```

On PowerShell:

```powershell
$env:CODE_RCL_TEST_PRECISE = "1"
cargo test --test resolve_accuracy -- --ignored --nocapture
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

CI runs on pushes and pull requests on Windows, Linux, and macOS. It checks
formatting, Clippy, tests, and builds with/without the viewer. The release
workflow calls those checks before building/publishing tagged binaries.
A separate real-LSP job runs on manual CI dispatch with the required servers.

Before submitting changes:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --no-default-features
cargo build --locked --no-default-features
```

The viewer integration test is gated by the viewer feature. Real-LSP tests are
optional; mandatory mock-LSP tests still run in both feature configurations.
If a temporary directory cannot be created/read, retain the error and inspect
its path and permissions; do not turn the failure into a passing skipped test.

Benchmarks are opt-in through CODE_RCL_RUN_BENCHMARKS=1 and are documented in
[BENCHMARKS.md](BENCHMARKS.md); performance runs are separate from correctness CI.

## Embedding tests

Default CI uses deterministic mock embedding vectors and token offsets to test
chunk budgets/Unicode/overlap, incremental reuse/invalidation, partial results,
source/configuration races, cancellation/resume, schema validation and CLI/MCP.
It does not fetch a model during ordinary unit/integration tests.

Real-model tests require the pinned six-file model directory:

```powershell
$env:CODE_RCL_EMBEDDING_MODEL_DIR = "C:\Models\embeddinggemma-1"
cargo test --bin code-rcl embedding_model_accuracy -- --ignored --nocapture
cargo test --test embeddings_cli real_model_setup_index_search_and_partial_update -- --ignored --nocapture
```

The separate model CI job prepares the model explicitly, runs these tests and
retains logs. The frozen Rust-only dataset and historical rankings are under
`tests/fixtures/embedding_accuracy`. Its paired translations are 16 tasks, not
32 independent tasks. This checks regression under a fixed configuration; it is
not a general accuracy claim. Document/query inference costs are logged apart.

The accuracy fixture retains historical input formatting for an apples-to-apples
model-runtime regression. Production owner/signature/doc chunk assembly and
source ranges are validated by mock service tests and real CLI/MCP integration
tests; the frozen fixture is not a full-production retrieval benchmark.

## Operational regressions and pinned retrieval corpus

Normal tests cover heartbeat without completed work, status under an exclusive
cache lock, interrupted snapshots, parser health on warm cache, the reduced
Kotlin grammar fixture, small output budgets retaining symbol identities, and
CLI/MCP selection parity. The BknDb 80-query corpus checks revision/hashes,
labels and task-disjoint 56/24 splits without loading a model.

The separate real-model runner also invokes `scripts/test-embedding-cancellation.py`:
it sends a real console interrupt (hidden isolated console on Windows), checks
retained file commits, resumes indexing, and verifies semantic search. It never
signals the user's terminal. Failure to deliver an interrupt is a test failure.

See [retrieval evaluation](RETRIEVAL-EVALUATION.md) for the full-corpus runner,
offline ranking experiments, profiling and interpretation limits.

Manual CI dispatch can enable `retrieval_benchmark` to run the full frozen corpus
with the prepared model and upload per-query artifacts on each model-job OS.
This expensive benchmark remains separate from normal correctness tests.

## Indexing-speed regression harness

Mock regressions cover length buckets/fairness, cross-file ordering, duplicate
fan-out, prior-vector reuse, source/config races, invalid model counts/vectors,
staging/commit rollback, cancellation/resume, and owned-staging recovery.
AST compact tests cover nested containers, Unicode, braces in strings/comments,
field initializers, abstract members, fallback and pipeline migration.
Use [INDEXING-BENCHMARK.md](INDEXING-BENCHMARK.md) for explicit real-model tests.
Retain clean source-byte hashes and full rankings, not only summary scores.
