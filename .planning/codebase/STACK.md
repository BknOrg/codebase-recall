---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# Technology Stack

**Analysis Date:** 2026-09-21

## Languages

**Primary:**

- Rust (edition 2024) - entire CLI in `src/`; binary `code-rcl` (`src/main.rs`)

**Secondary:**

- JavaScript/CSS - embedded web graph UI in `src/assets/` (`live.js`, `graph.css`, vendored `d3.min.js`)
- Shell/PowerShell - installers in `scripts/codebase-recall-installer.sh` and `.ps1`

**Analyzed languages (tree-sitter):** Rust, JavaScript, TypeScript, Python, Java, Kotlin, Go, plus TOML (`src/analysis/toml.rs`).

## Runtime

**Environment:**

- Native compiled binary, no runtime needed

**Package Manager:**

- Cargo
- Lockfile: present (`Cargo.lock`)

## Frameworks

**Core:**

- clap 4.5 (derive) - CLI parsing (`src/cli.rs`)
- tree-sitter 0.25 + grammars (rust 0.24, javascript/typescript/python/java/go 0.23, kotlin-ng 1.1) - AST analysis (`src/analysis/`)
- rusqlite 0.32 (bundled SQLite) - graph cache (`src/cache/`)
- tiny_http 0.12 - local web server (`src/server/mod.rs`)

**Testing:**

- Rust built-in `cargo test`; integration tests in `tests/*.rs` with fixtures in `tests/fixtures/`

**Build/Dev:**

- cargo-dist style `[profile.dist]` (release + thin LTO) in `Cargo.toml`
- GitHub Actions release workflow `.github/workflows/release.yml`

## Key Dependencies

**Critical:**

- `blake3` 1 - content hashing for cache invalidation
- `ignore` 0.4, `globset` 0.4 - gitignore-aware file walking and filtering
- `serde` / `serde_json` 1 - serialization; MCP JSON-RPC (`src/commands/mcp/`)
- `anyhow` 1.0 - error handling

**Infrastructure:**

- `webbrowser` 1.2 - opens graph UI
- `ctrlc` 3.5 - graceful shutdown of server

## Configuration

**Environment:**

- `CODE_RCL_LSP_{RUST,PYTHON,JAVA,KOTLIN,TYPESCRIPT,JAVASCRIPT,GO}` override language-server paths (`src/precise/backend.rs`)
- Also reads `HOME`, `USERPROFILE`, `PATHEXT`
- `.mcp.json` registers `code-rcl mcp` as an MCP server

**Build:**

- `Cargo.toml`, `Cargo.lock`

## Platform Requirements

**Development:**

- Stable Rust toolchain supporting edition 2024; C compiler for bundled SQLite and tree-sitter grammars

**Production:**

- Prebuilt binaries: linux-musl x86_64, windows-msvc x86_64, macOS x86_64 and aarch64
- Project state stored in `<project>/.code-rcl/cache.db`

---

*Stack analysis: 2026-09-21*
