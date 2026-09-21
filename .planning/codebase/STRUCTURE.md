---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# Codebase Structure

**Analysis Date:** 2026-09-21

## Directory Layout

```
code-reviewer/
├── Cargo.toml          # crate `codebase-recall`, bin `code-rcl`, edition 2024
├── src/
│   ├── main.rs         # entry + dispatch
│   ├── cli.rs          # clap definitions
│   ├── service.rs      # shared sync/query helpers
│   ├── analysis/       # tree-sitter extractors (go, java, javascript, kotlin, python/, rust/, sfc, toml, scope, lang)
│   ├── cache/          # SQLite: schema, models, queries, mutations, mappers, utils
│   ├── commands/       # one module per subcommand (digest/, impact/, explain/, mcp/ are dirs)
│   ├── graph/          # CodeGraph, community, query, render/, resolve/
│   ├── dump/           # walker, formatter
│   ├── precise/        # LSP backends/client
│   ├── server/         # local HTTP server
│   └── assets/         # embedded web UI (graph/NN-*.js, graph.css, d3, icons)
├── tests/              # integration tests + fixtures/
├── skills/code-rcl/    # AI agent skill
├── scripts/            # installer .ps1/.sh
├── docs/media/         # README media
└── CODE_RCL_ANALYSIS_ISSUES_REPORT.md
```

## Key File Locations

**Entry Points:** `src/main.rs`
**Configuration:** `Cargo.toml`
**Core Logic:** `src/commands/sync.rs`, `src/graph/resolve/`, `src/analysis/`
**Schema:** `src/cache/schema.rs`
**Testing:** `tests/*.rs` (per-command `*_cli.rs`, `resolve_accuracy.rs`, `type_edges.rs`, `macro_bodies.rs`, `config_keys.rs`), fixtures in `tests/fixtures/<name>_app/`

## Naming Conventions

**Files:** snake_case `.rs`; language analyzers named after language; web JS ordered by numeric prefix (`50-render.js`).
**Directories:** snake_case; complex modules use `mod.rs` + `models.rs`/`render.rs`.
**Fixtures:** `<topic>_app/`, optional `expected-edges.json`.

## Where to Add New Code

**New subcommand:** args in `src/cli.rs`, `Command` variant, module in `src/commands/`, register in `src/commands/mod.rs` and `src/main.rs` match.
**New language:** analyzer in `src/analysis/`, register in `src/analysis/mod.rs` and `lang.rs`, add tree-sitter dep to `Cargo.toml`.
**New DB column/table:** append a migration in `src/cache/schema.rs` and bump `SCHEMA_VERSION`.
**New MCP tool:** `src/commands/mcp/tools.rs` + `handlers.rs`.
**Tests:** `tests/<name>.rs` with fixture in `tests/fixtures/<name>_app/`.
**Shared helpers:** `src/service.rs`.

## Special Directories

**`.code-rcl/`:** per-analyzed-project cache (`cache.db`, `REPORT.md`); generated, not committed.
**`target/`:** build output; not committed.

---

*Structure analysis: 2026-09-21*
