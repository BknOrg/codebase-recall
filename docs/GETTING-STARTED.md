<!-- generated-by: gsd-doc-writer -->
# Getting Started

This guide takes you from a fresh machine to a working `code-rcl` (codebase-recall) run.

## Prerequisites

- **Rust toolchain >= 1.85** (the crate uses `edition = "2024"`). Only needed for `cargo install` or building from source. Install via [rustup](https://rustup.rs).
- **Git** to clone the repository (and for `code-rcl impact --diff`).
- Optional, for `--precise` mode only: language servers for your languages (e.g. `rust-analyzer`, `pyright`, `typescript-language-server`, `gopls`). See the README for the full list.

Prebuilt binaries need no Rust toolchain.

## Installation

### Option A: Prebuilt binary

macOS / Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.ps1 | iex
```

### Option B: From crates.io

```bash
cargo install codebase-recall
```

### Option C: From source

1. Clone the repository:

   ```bash
   git clone https://github.com/BknOrg/codebase-recall.git
   ```

2. Enter the project directory:

   ```bash
   cd codebase-recall
   ```

3. Build and install:

   ```bash
   cargo build --release
   cargo install --path .
   ```

The installed binary is named `code-rcl`. Verify with:

```bash
code-rcl --help
```

## First Run

From the root of any project you want to analyze:

```bash
cd path/to/your/project
code-rcl init     # creates .code-rcl/ (cache.db, config.toml) and updates .gitignore
code-rcl serve    # syncs, then opens the interactive graph in your browser
```

Closing the browser tab stops the server automatically. To produce output without a browser:

```bash
code-rcl dump . -o context.md   # Markdown bundle of the codebase for an LLM
code-rcl digest                 # architecture outline to stdout
```

## Common Setup Issues

1. **`code-rcl: command not found` after `cargo install`.** Cargo's bin directory (`~/.cargo/bin`, or `%USERPROFILE%\.cargo\bin` on Windows) is not on your `PATH`. Add it and restart the shell.
2. **Build fails with an edition or unstable-feature error.** Your Rust toolchain is too old for edition 2024. Run `rustup update stable`.
3. **`--precise` reports a missing language server.** Nothing is bundled; install the server for that language (the error message includes the install command). Without it, that language keeps its heuristic edges.
4. **Graph or dump seems stale.** Commands auto-sync by default; if you passed `--no-sync`, run `code-rcl sync` to refresh the cache in `.code-rcl/cache.db`.
5. **Files missing from a dump.** Files matched by `.gitignore`, binaries, lockfiles, `.env*` files, and files over the size limit (default 50 KB for `dump`) are skipped. Raise it with `--max-size-kb`.

## Next Steps

- [ARCHITECTURE.md](ARCHITECTURE.md) - how the parser, resolver, cache, and commands fit together.
- [CONFIGURATION.md](CONFIGURATION.md) - `.code-rcl/config.toml` and environment variables.
- [README.md](../README.md) - full command reference (`impact`, `path`, `explain`, `report`, `mcp`, `setup`).
