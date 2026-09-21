---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
<!-- refreshed: 2026-09-21 -->

# Architecture

**Analysis Date:** 2026-09-21

## System Overview

```text
┌─────────────────────────────────────────────────────────────┐
│  CLI (clap)  `src/cli.rs`  ->  dispatch `src/main.rs`        │
├───────────┬───────────┬───────────┬───────────┬─────────────┤
│ sync/init │ digest/   │ graph/    │ dump/     │ mcp/setup   │
│           │ impact/   │ serve/    │ search    │ (JSON-RPC)  │
│           │ path/     │ report    │           │             │
│           │ explain   │           │           │             │
│           `src/commands/*`                                   │
└────────┬──────────┬───────────┬───────────────┬─────────────┘
         ▼          ▼           ▼               ▼
┌─────────────────────────────────────────────────────────────┐
│ `src/service.rs` (auto_sync + graph query helpers)           │
├─────────────────────────────────────────────────────────────┤
│ `src/graph/` resolve -> CodeGraph -> community/query/render  │
├─────────────────────────────────────────────────────────────┤
│ `src/cache/` SQLite   |  `src/analysis/` tree-sitter parsers │
│                       |  `src/precise/` LSP ground truth     │
└─────────────────────────────────────────────────────────────┘
   `<project>/.code-rcl/cache.db`, `REPORT.md`
```

## Component Responsibilities

| Component | Responsibility | File |
|-----------|----------------|------|
| CLI definition | clap args/subcommands | `src/cli.rs` |
| Dispatch | Normalizes `help`, routes to commands | `src/main.rs` |
| Sync | Incremental parse -> cache | `src/commands/sync.rs` |
| Analysis | Per-language symbol/import/ref/scope extraction | `src/analysis/` |
| Cache | SQLite schema, migrations, queries, mutations | `src/cache/` |
| Graph resolver | Layered ref resolution into edges | `src/graph/resolve/` |
| Communities | Subsystem detection | `src/graph/community.rs` |
| Renderers | dot/html/json | `src/graph/render/` |
| Precise | Optional LSP (rust-analyzer, Pyright, JDT, kotlin-ls) | `src/precise/` |
| Server | Local tiny_http graph viewer | `src/server/mod.rs`, `src/commands/serve.rs` |
| MCP | JSON-RPC stdio tools | `src/commands/mcp/` |
| Dump | Context bundle walker/formatter | `src/dump/` |
| Assets | Embedded web UI (d3, graph JS/CSS) | `src/assets/` |

## Pattern Overview

**Overall:** Single-binary (`code-rcl`) layered CLI; cache-then-graph pipeline.

**Key Characteristics:**

- Incremental sync keyed by blake3 content hash into SQLite.
- Heuristic edges from AST; optional `--precise` LSP overrides.
- Read-only commands auto-sync first via `src/service.rs`.

## Layers

**Commands:** `src/commands/`; depends on service, graph, cache; used by `src/main.rs`.
**Service:** `src/service.rs`; shared sync + `GraphQuery` builders.
**Graph:** `src/graph/`; builds `CodeGraph` (nodes/edges/communities) from cache.
**Analysis:** `src/analysis/`; produces `ParsedFile` (symbols, imports, refs, scopes, bindings, string literals). Languages: Rust, Python (+ipynb), JS/TS (+sfc), Java, Kotlin, Go, TOML (config keys).
**Cache:** `src/cache/`; `SCHEMA_VERSION` 5 forward-only migrations in `src/cache/schema.rs`.

## Data Flow

### Primary Request Path

1. `main.rs` parses `Cli`, dispatches to `commands::<cmd>::run`.
2. `service::auto_sync` -> `commands::sync::sync_cache` walks files (`ignore`), hashes, parses changed files via `analysis`, writes cache.
3. `graph::resolve` reads cache, resolves refs (imports, scope, types, precise) to edges -> `CodeGraph`.
4. Command renders result (markdown digest, impact tree, graph, JSON).

### Serve Flow

1. `commands/serve.rs` builds graph, `server/mod.rs` serves embedded assets on 127.0.0.1.
2. `/live` EventSource keeps alive; server exits when tab closes, `/quit`, or Ctrl-C.

**State Management:** Persistent state only in `.code-rcl/cache.db`.

## Key Abstractions

- `ParsedFile` in `src/analysis/mod.rs`; `Language` in `src/analysis/lang.rs`.
- `CodeGraph`/`Node`/`Edge` in `src/graph/mod.rs`.
- `GraphQuery`, `SyncArgs`, `PreciseArgs` in `src/cli.rs`.
- `CacheDb` in `src/cache/mod.rs`.

## Entry Points

- `src/main.rs` (binary `code-rcl`); `commands::mcp::run` for stdio MCP.

## Architectural Constraints

- **Threading:** Mostly synchronous; `serve` uses threads and atomics.
- **Global state:** None significant; embedded assets in `src/assets/mod.rs`.
- **Schema:** Migration count must equal `SCHEMA_VERSION`.

## Anti-Patterns

### Duplicating sync/graph setup

**What happens:** Commands rebuilding `SyncArgs`/`GraphQuery` by hand.
**Do this instead:** Use `src/service.rs` helpers.

## Error Handling

**Strategy:** `anyhow::Result` propagated to `main`; missing LSP servers skipped with install hints.

## Cross-Cutting Concerns

**Logging:** stderr/stdout prints. **Validation:** clap. **Authentication:** none.

---

*Architecture analysis: 2026-09-21*
