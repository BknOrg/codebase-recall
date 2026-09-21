---
last_mapped_commit: d7806bd580d058435c1ef3a34f15fae2882bb802
last_mapped_at: 2026-09-21
---
# External Integrations

**Analysis Date:** 2026-09-21

## APIs & External Services

**Language servers (optional, `--precise`):**

- User-installed LSP servers launched over stdio: rust-analyzer, pyright/basedpyright-langserver, jdtls, kotlin-language-server, typescript-language-server/vtsls, gopls
  - Client: `src/precise/client.rs`, `src/precise/backend.rs`, `src/precise/map.rs`
  - Auth: none; path overrides via `CODE_RCL_LSP_*` env vars

**AI agent integration:**

- MCP server (`code-rcl mcp`) - `src/commands/mcp/handlers.rs`, `src/commands/mcp/tools.rs`; registered in `.mcp.json`
- Agent skill definition in `skills/code-rcl/`; setup helper `src/commands/setup_integrations.rs`

**Git:**

- Shells out to `git` CLI (`src/commands/impact/diffscan.rs`, `src/commands/setup_integrations.rs`)

No third-party network APIs detected.

## Data Storage

**Databases:**

- SQLite (bundled) at `<project>/.code-rcl/cache.db`
  - Client: rusqlite; schema in `src/cache/schema.rs`

**File Storage:** Local filesystem only

**Caching:** SQLite graph cache (`src/cache/`)

## Authentication & Identity

**Auth Provider:** None. Web UI binds to `127.0.0.1` only (`src/server/mod.rs`).

## Monitoring & Observability

**Error Tracking:** None
**Logs:** stdout/stderr output only

## CI/CD & Deployment

**Hosting:** GitHub releases (`BknOrg/codebase-recall`)

**CI Pipeline:** GitHub Actions `.github/workflows/release.yml`, triggered on `v*` tags; builds four targets and publishes archives with sha256 files. Installers in `scripts/`.

## Environment Configuration

**Required env vars:** None required; optional `CODE_RCL_LSP_*`.

**Secrets location:** Not applicable

## Webhooks & Callbacks

**Incoming:** Local HTTP endpoints for the graph UI (`src/server/mod.rs`)
**Outgoing:** None

---

*Integration audit: 2026-09-21*
