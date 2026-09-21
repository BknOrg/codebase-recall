<!-- generated-by: gsd-doc-writer -->
# Development

This guide covers working on `codebase-recall` (binary: `code-rcl`) itself. For module layout see [ARCHITECTURE.md](ARCHITECTURE.md); for settings see [CONFIGURATION.md](CONFIGURATION.md).

## Local setup

Prerequisites: a Rust toolchain that supports the 2024 edition (`edition = "2024"` in `Cargo.toml`). SQLite is bundled via `rusqlite`'s `bundled` feature, so no system SQLite is needed. The tree-sitter grammars are compiled from crates, so a working C compiler is required.

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
- The release workflow (`.github/workflows/release.yml`) only builds binaries on `v*` tags; it does not run lint or format checks, so run both locally before opening a PR.

## Testing

Integration tests are in `tests/` (for example `digest_cli.rs`, `graph_cli.rs`, `impact_cli.rs`, `resolve_accuracy.rs`) with sample projects in `tests/fixtures/`. Run everything with `cargo test`, or one file with `cargo test --test digest_cli`.

## Branch conventions

No convention is documented. The default branch is `main`. Suggested pattern: short descriptive branches such as `feat/my-feature` or `fix/my-bug`.

## PR process

No PR template exists in the repository; see [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines. Reasonable expectations:

- Branch from `main` and keep the change focused.
- Run `cargo fmt`, `cargo clippy`, and `cargo test` before pushing.
- Add or update tests in `tests/` for behavior changes.
- Update `README.md` when adding or changing CLI flags or subcommands.
- Releases are produced by pushing a `v*` tag, which triggers the Release workflow.
