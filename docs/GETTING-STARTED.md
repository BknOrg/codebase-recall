# Getting started with code-rcl 0.9.1

Use [the usage guide](USAGE.md) for everyday CLI commands and
[the MCP guide](MCP.md) for agent integration. Existing integrations should read
[the migration guide](MIGRATION-0.9.md) before upgrading.

## Install the version you intend to use

Prebuilt releases need no Rust toolchain. Building this checkout requires a
current stable Rust toolchain and a C compiler for tree-sitter grammars. Git is
needed for cloning and `impact --diff`. Language servers are optional until you
request precise resolution.

To install the implementation in this checkout:

```sh
cargo install --path . --locked
code-rcl --version
```

Check that the reported version is `0.9.1`. To run directly from source without
replacing an installed binary:

```sh
cargo run -- --version
cargo run -- report --project /absolute/path/to/project
```

For published releases, use `cargo install codebase-recall` or the release
installers linked from the [README](../README.md#installation). Those install
published artifacts; they do not include unpublished working-tree changes.

## First analysis

Run these commands from the project you want to inspect:

```sh
code-rcl init
code-rcl report
code-rcl search CacheDb --match-mode exact
code-rcl explain CacheDb
code-rcl impact CacheDb --depth 2
```

Replace `CacheDb` with a symbol from your project. `init` creates configuration
and cache state and excludes `.code-rcl/` locally in Git. Analysis commands can
create the cache themselves if needed. Normal sync hashes eligible files and
parses changed contents; `--no-sync` explicitly uses unverified cached data.

## Export or view results

```sh
code-rcl graph --format json -o graph.json
code-rcl dump -r CacheDb --depth 2 -o context.md
code-rcl serve
```

`serve` opens the browser viewer. Closing its connected tab stops the server.
`--no-open` prints a URL you can open yourself; the server times out if no tab
connects. Viewer exports create new files and do not overwrite existing ones.

## Install the agent skill and configure MCP

After installing the new binary, run from the target project:

```sh
code-rcl setup --workspace
```

This installs `.agents/skills/code-rcl/SKILL.md` and its `references/` bundle.
Built-in MCP configuration targets are Claude and Gemini/Antigravity; other
clients need their own MCP configuration format. See [MCP setup](MCP.md#setup).
Restart or reconnect your client after updating its binary/configuration.
An installed global skill is a separate copy and is not updated merely by
editing this repository.

## Troubleshooting

- **Old behavior after updating source:** check `code-rcl --version` and the
  resolved executable (`Get-Command code-rcl` on PowerShell, `command -v
  code-rcl` on Unix). Install this checkout or use `cargo run -- ...`.
- **Precise fails:** install/configure the reported language server and use a
  valid project manifest. Fallback is opt-in through
  `--precise --allow-heuristic-fallback`; inspect warnings and partial status.
- **Files are absent:** check ignore rules, supported extensions, language
  filters, and size limits. Dump defaults to 50 KiB per file.
- **Cache is locked:** finish the other process using that project and retry.
- **MCP rejects `json`:** remove that parameter. MCP always returns structured
  results in 0.9.1.
- **MCP requires a project:** configure the server with `--project` or supply
  `project` in the tool call.

See [configuration](CONFIGURATION.md), [testing](TESTING.md), and
[architecture](ARCHITECTURE.md) for more detail.

## Semantic and hybrid retrieval

See the [embedding usage guide](EMBEDDINGS.md) for explicit model setup, indexing,
semantic/hybrid search, partial results, build variants and troubleshooting.
