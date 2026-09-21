<!-- generated-by: gsd-doc-writer -->
# Contributing to codebase-recall

Thanks for your interest in contributing to `codebase-recall` (`code-rcl`), a Rust CLI for dumping codebase context for LLMs and graphing code relations.

## Development Setup

This is a Rust (edition 2024) project built with Cargo. Clone the repository and build:

```bash
git clone https://github.com/BknOrg/codebase-recall.git
cd codebase-recall
cargo build
```

The binary is `code-rcl` (source entry point: `src/main.rs`). See [README.md](README.md) for usage, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the modules fit together, and [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for configuration options.

## Coding Standards

- No custom formatter or linter configuration is checked into the repository, so use the Rust defaults.
- Format code with `cargo fmt` before committing.
- Run `cargo clippy` and address warnings in the code you touch.
- CI (`.github/workflows/release.yml`) only builds release artifacts; it does not enforce formatting or linting, so please check locally.

## Testing

Integration tests live in `tests/` (with fixtures in `tests/fixtures/`). Run them with:

```bash
cargo test
```

Add or update tests for any behavior change. Follow the existing `tests/<area>_cli.rs` naming pattern for CLI-level tests.

## Pull Request Guidelines

- No PR template or branch naming convention is documented; use short, descriptive branch names (for example `feat/my-feature` or `fix/my-bug`).
- Keep each PR focused on a single change.
- Make sure `cargo build`, `cargo test`, and `cargo fmt --check` pass locally.
- Update the README or `docs/` when you change user-facing behavior, CLI flags, or configuration.
- Describe what changed and why in the PR description, and link related issues.

## Issue Reporting

Report bugs and request features through [GitHub Issues](https://github.com/BknOrg/codebase-recall/issues). No issue templates exist, so please include:

- Steps to reproduce the problem
- Expected and actual behavior
- Your OS and the `code-rcl --version` output
- The command you ran and any relevant output

## License

By contributing, you agree that your contributions are licensed under the project's [MIT License](LICENSE).
