# Development

This guide covers working on `codebase-recall` (binary: `code-rcl`) itself. For module layout see [ARCHITECTURE.md](ARCHITECTURE.md); for settings see [CONFIGURATION.md](CONFIGURATION.md).

## Local setup

Prerequisites: a Rust toolchain that supports the 2024 edition (`edition = "2024"` in `Cargo.toml`). The cache is an embedded, pure-Rust `bkndb` database, so no system database library is needed. The tree-sitter grammars are compiled from crates, so a working C compiler is required.

1. Fork and clone the repository:

   ```bash
   git clone https://github.com/BknOrg/codebase-recall.git
   cd codebase-recall
   ```

2. Build:

   ```bash
   cargo build
   ```

3. Run the binary from source:

   ```bash
   cargo run -- --help
   cargo run -- digest
   ```

4. (Optional) Install your local build as `code-rcl`:

   ```bash
   cargo install --path .
   ```

No `.env` file is needed. Per-project settings live in `.code-rcl/config.toml`, created by `code-rcl init`. The `.code-rcl/` directory is git-ignored.

Optional: `code-rcl sync --precise` and `--precise` on other commands use external language servers (for example `rust-analyzer`). They are only needed when working on the `src/precise` code or its tests.

## Build commands

The project uses standard Cargo commands; there is no Makefile. The `scripts/` directory holds only the `codebase-recall-installer` scripts (`.sh` and `.ps1`), not build tooling.

| Command | Description |
| :--- | :--- |
| `cargo build` | Debug build |
| `cargo build --release` | Optimized release build |
| `cargo run -- <args>` | Run `code-rcl` with arguments |
| `cargo test` | Run unit and integration tests |
| `cargo fmt` | Format code with rustfmt |
| `cargo clippy` | Lint with Clippy |
| `cargo build --profile dist` | Release build with thin LTO (the `dist` profile in `Cargo.toml`) |

## Code style

- **rustfmt** for formatting: `cargo fmt`. No `rustfmt.toml` is present, so defaults apply.
- **Clippy** for linting: `cargo clippy`. No `clippy.toml` is present.
- CI runs format, Clippy, tests and no-viewer builds across Windows, Linux and macOS. The release workflow waits for the reusable CI checks before packaging binaries.

## Testing

Integration tests are in `tests/` (for example `digest_cli.rs`, `graph_cli.rs`, `impact_cli.rs`, `resolve_accuracy.rs`) with sample projects in `tests/fixtures/`. Run everything with `cargo test --all-targets`, or one file with `cargo test --test digest_cli`. Run the no-default-features configuration too; see [TESTING.md](TESTING.md). The native mock LSP is mandatory, while real-LSP tests are opt-in.

## Branch conventions

No convention is documented. The default branch is `main`. Suggested pattern: short descriptive branches such as `feat/my-feature` or `fix/my-bug`.

## PR process

No PR template exists in the repository; see [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines. Reasonable expectations:

- Branch from `main` and keep the change focused.
- Run `cargo fmt`, `cargo clippy`, and `cargo test` before pushing.
- Add or update tests in `tests/` for behavior changes.
- Update `README.md` when adding or changing CLI flags or subcommands.
- Releases are produced by pushing a `v*` tag, which triggers the Release workflow.

## Changing the skill or MCP contract

The maintained skill is skills/code-rcl/SKILL.md, with conditional references
under references/. Setup embeds and installs that entire bundle. Check example
commands against the current CLI and tool arguments against tools/list; the
input schemas are generated from the MCP input types.

Do not verify source changes using an older installed binary. Use cargo run or
the integration tests' CARGO_BIN_EXE_code-rcl. Editing source does not update
installed skills, binaries, or client processes. Follow [USAGE.md](USAGE.md#update-an-installed-skill-or-mcp-server)
when intentionally updating an installation.

Document public contract changes in [MIGRATION-0.9.md](MIGRATION-0.9.md). JSON CLI
results and MCP structuredContent share the envelope in output.rs. Keep stdout
free of user-facing diagnostics in functions reachable from MCP; use stderr or
structured metadata instead.

## Portable compiler configuration

The tracked Cargo configuration uses host toolchain discovery. On Windows,
install Visual Studio C++ Build Tools and a Windows SDK; a Developer PowerShell
session can provide LIB/INCLUDE when needed. Keep absolute machine paths in an
untracked local config and opt in explicitly:

```powershell
cargo --config .cargo/config.windows-local.toml build
```

The previous development-machine settings are preserved locally at that path;
they are not shipped to other machines or used by CI.

Use [operations](OPERATIONS.md) for the shared progress/cancellation contract.
Do not weaken freshness checks for benchmarks; use isolated frozen fixtures.
New retrieval experiments remain offline until held-out regressions and costs
are reviewed; see [evaluation](RETRIEVAL-EVALUATION.md).
