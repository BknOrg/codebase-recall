# MCP usage — 0.9.1

code-rcl exposes nine tools over newline-delimited JSON-RPC on stdio, using
protocol version `2025-06-18`. stdout contains protocol messages only; diagnostic
logging goes to stderr. Normal clients manage initialization and tool discovery.

## Setup

First install the current executable from the code-rcl checkout:

```sh
cargo install --path . --locked
code-rcl --version
```

Editing this repository does not update an installed binary or skill copy.
Check the executable used by your client: `--help` must list `uninstall`.
A version number alone cannot distinguish uncommitted checkout changes.

Run from the project you want to analyze:

```sh
code-rcl setup --workspace
```

Setup installs the full skill bundle (including references) and native MCP
configuration through the same command for every agent. Default scope is the
workspace; default target `all` includes all three agents.

| Target | Workspace skill | Workspace MCP | Global skill | Global MCP |
| --- | --- | --- | --- | --- |
| Codex | `.agents/skills/code-rcl` | `.codex/config.toml` | `~/.agents/skills/code-rcl` | `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`) |
| Claude Code | `.claude/skills/code-rcl` | `.mcp.json` | `~/.claude/skills/code-rcl` | `~/.claude.json` |
| Antigravity/Gemini | `.agents/skills/code-rcl` | `.agents/mcp_config.json` | `~/.gemini/config/skills/code-rcl` | `~/.gemini/config/mcp_config.json` |

Select `--target codex`, `--target claude`, or `--target antigravity`;
`--target gemini` is an alias for Antigravity. Codex uses
`[mcp_servers.code-rcl]` in TOML, not the JSON `mcpServers` key.
Workspace servers receive an absolute `--project` root. Codex project config
loads only in trusted workspaces; setup never changes trust settings.
Restart/reconnect the client after setup to reload MCP.

Setup preflights all selected resources before writing. Invalid JSON/TOML or
malformed managed markers leave destinations unchanged. TOML comments and
unrelated agent settings are retained. Files are replaced atomically; an I/O
failure during application reports partial progress and can be retried.

Manual configuration for clients that use `mcpServers`:

```json
{
  "mcpServers": {
    "code-rcl": {
      "command": "code-rcl",
      "args": ["mcp", "--project", "/absolute/path/to/project"]
    }
  }
}
```

Use an absolute executable path when your client's PATH differs from your
terminal. On Windows, project paths may be written as `C:/work/my-project` in
JSON. Follow your client's own configuration format when it does not use
`mcpServers`.

Useful setup variants:

```sh
code-rcl setup --workspace --skill-only
code-rcl setup --workspace --mcp-only
code-rcl setup --workspace --instructions
code-rcl setup --print-skill
```

`--instructions` adds marked guidance to supported instruction files. Git and
Claude session hooks are separate opt-ins (`--git-hook`, `--claude-hook`).
`setup --workspace --remove` keeps its legacy behavior: it removes only those
optional instruction/hook integrations. Use the new command for full removal:

```sh
code-rcl uninstall --workspace --target codex
code-rcl uninstall --global --target all
code-rcl uninstall --workspace --target claude --mcp-only
code-rcl uninstall --workspace --target codex --skill-only --skill-dir custom-skill
```

Uninstall removes skill/MCP plus related managed instructions/hooks by default;
component selectors remove only that component. Its target/scope rules match
setup. The executable, local model and project index are retained. Only the
`code-rcl` server entry, known bundle files and marked blocks/hooks are removed;
user-added skill files and unrelated configuration remain.

Ownership is recorded in `.code-rcl/agent-integrations.json` under the workspace
or user profile. Workspace Codex/Antigravity share a skill and AGENTS.md block;
uninstalling one retains shared resources while another owner remains. A git
hook is retained until its last owner is removed. Legacy shared resources with
unknown owners are retained for single-target removal, with an explanation;
`--target all` can remove recognized legacy bundles. Unknown/custom files are
not recursively deleted. Repeat `--skill-dir` when removing a custom bundle.
Use the same scope during removal: workspace uninstall does not erase a global
registration, and global uninstall does not alter other projects.

Global configuration is explicit with `setup --global`. It does not pin a
project: supply `project` in tool calls, or manually launch the server with
`--project`. After installing/configuring the new version, restart or reconnect
the client so it reloads the executable, tool schemas, and installed skill.

## Tool arguments

All tools accept:

| Argument | Default | Meaning |
| --- | --- | --- |
| `project` | server's `--project` | Existing project directory; required if server has no root. Prefer an absolute path. |
| `no_sync` | `false` | Use cached data without refresh; reports unverified freshness. Cannot be true for sync or with either precision mode. |
| `max_output_kb` | `256` | Serialized result budget in KiB, range 1–2048; includes structured and text representations. |

There is no MCP `json` argument in 0.9.1. Results are always structured.
Unknown arguments, wrong types, empty required strings, and invalid ranges are
rejected. Read `tools/list` for the schema generated by the running binary.

| Tool | Specific arguments |
| --- | --- |
| `code_rcl_report` | Architecture overview; optional `languages`, `paths`, `exclude_paths`, `categories`. |
| `code_rcl_digest` | `path`, `all`, `with_docs`, `doc_lines`. |
| `code_rcl_search` | Required `query`; optional `languages`, `paths`, `exclude_paths`, `categories`, `retrieval` (`auto`, `lexical`, `semantic`, `hybrid`; default lexical until release gates pass), `match_mode` (`exact`, `prefix`, `substring`, default substring), `kind`, `exported_only`, `strings`, `limit` (1–100, default 25). |
| `code_rcl_impact` | Exactly one of `symbol` or `diff: true`; `depth` (1–8, default 2), `direction` (both/reverse/forward), `kinds`, `precise` OR `precise_selective`, `allow_heuristic_fallback`. |
| `code_rcl_path` | Required `from`, `to`; `max_depth` (1–20, default 8), `direction` (forward/reverse/any), `kinds`, `precise` OR `precise_selective`, `allow_heuristic_fallback`. |
| `code_rcl_explain` | Required `symbol`; `precise` OR `precise_selective`, `allow_heuristic_fallback`. |
| `code_rcl_dump` | Required `target`; `depth` (1–5, default 2), `max_size_kb` per file (1–10240, default 50). |
| `code_rcl_sync` | `languages`, `embeddings` (default false), `precise` OR `precise_selective`, `full`, `allow_heuristic_fallback`. Languages limits source/LSP scope; full forces the selected precise policy and/or embeddings. |
| `code_rcl_graph` | `scope` (file/symbol/both), `focus`, `depth` (1–5, default 2), `max_nodes` (1–100000, default 2000), `kinds` (default imports/calls/contains), `min_confidence` (0–1, default 0), `precise` OR `precise_selective`, `allow_heuristic_fallback`. |

For impact/path, default kinds are calls, imports, and references.
Graph/impact/path kinds accept calls, imports, references, contains, inherits,
implements and overrides. Structural edges require explicit selection in MCP.
For example, call `code_rcl_graph` with
`{"kinds":["calls","inherits","overrides"],"min_confidence":0.5}` to inspect
structural relationships while hiding ambiguous guesses (capped at 0.35).
Generated tool schemas enumerate the same edge kinds validated at runtime.
`precise` and `precise_selective` are mutually exclusive. Selective mode gracefully
retains remaining heuristics on errors; strict full mode needs explicit
`allow_heuristic_fallback: true` for partial output. Fallback requires a mode.
For MCP sync/graph, omitted mode flags follow `[precise] enabled` or `selective`
unless `no_sync` is true; an explicit mode field overrides those defaults.
`precise:false` disables configured mode selection for that request. Explain,
impact and path require explicit mode arguments. Report/search/digest consume
cached verification evidence and do not accept precision flags.

## Example protocol session

Each line below is one complete message written to the server's stdin. The
client must accept responses by ID because ping/list responses can arrive
before an earlier long analysis finishes.

```jsonl
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"example-client","version":"1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"code_rcl_search","arguments":{"query":"CacheDb","match_mode":"exact","limit":10}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"code_rcl_impact","arguments":{"symbol":"CacheDb","depth":2,"direction":"reverse"}}}
```

Launch with `code-rcl mcp --project /absolute/path/to/project` so the calls above
have a configured root. For precise impact, add `"precise": true` to the
arguments. Only add `"allow_heuristic_fallback": true` if partial results are
acceptable. For tracked changes, replace symbol with `"diff": true`; do not
send both. Diff excludes untracked files. Its historical graph uses AST heuristics even
when current code is resolved with LSP; do not treat the baseline as compiler-verified.

## Interpret results

Tool results contain `structuredContent` plus the same envelope serialized in
`content[0].text`. Use `structuredContent` directly rather than parsing the text:

```json
{
  "schema_version": 2,
  "status": "ok",
  "data": [],
  "metadata": {
    "project": "/absolute/path/to/project",
    "cache_generation": 1,
    "resolution_mode": "heuristic",
    "freshness": "verified",
    "warnings": [],
    "truncated": false,
    "omitted_count": 0,
    "returned_count": 0
  }
}
```

This illustrates an empty impact/path result; the data shape depends on the
tool. Graph data includes nodes/edges; search includes total_matches and result
lists; sync includes stats/totals; dump includes total_files and whole source
file entries under files.

A budget may omit complete array entries or files, never split JSON/source
text. Check truncated and omitted_count. `max_size_kb` limits one dump file;
`max_output_kb` limits the response. Increase the budget or narrow the query
when needed. Diff also limits output to the ten most affected connected targets.
Inspect warnings and parser coverage before treating an empty result as proof
that no relationship exists.

Malformed requests/invalid arguments produce JSON-RPC errors. Tool execution
failures return `isError: true` and an error envelope. Requested full precise failure
is an execution error unless fallback was explicit. Selective failures always
return partial results and recovery hints; completed policy coverage does not
mean every reference was LSP-verified. Inspect `metadata.precision.languages`.

## Cancellation and concurrency

Analysis jobs are serialized to protect the exclusive cache handle. The input
loop remains available for ping and cancellation:

```jsonl
{"jsonrpc":"2.0","id":5,"method":"ping"}
{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":4}}
```

Cancellation is cooperative at file, traversal, and LSP wait checkpoints.
Cancelled requests do not return result messages. Committed per-file cache
transactions remain usable; interrupted work is not stamped as a completed pass.

See [usage](USAGE.md), [configuration](CONFIGURATION.md), and
[migration](MIGRATION-0.9.md) for CLI behavior and compatibility changes.

## Embeddings in 0.9.1

The nine tool names and envelope version 1 are unchanged. `code_rcl_search` adds
`retrieval: auto|lexical|semantic|hybrid` (default lexical until automatic feature
retrieval passes release gates). Kind/exported filters
apply to both branches; match_mode configures the lexical branch. `strings:true`
is rejected for semantic/hybrid. A missing model never silently falls back.
Normal semantic/hybrid searches refresh pending in-scope embeddings; `no_sync`
uses saved, unverified state. Explicit auto task requests fail clearly while the
new pipeline is gated. See [search behavior and evaluation](AUTOMATIC-SEARCH.md).

`code_rcl_sync` adds `embeddings: true` for explicit indexing, after CLI
`code-rcl embeddings setup`. Combining embeddings with no_sync is invalid.
`full:true` requires `embeddings:true`, `precise:true` or `precise_selective:true`
(including configured sync modes) and refreshes only selected backends. Selective
force still selects uncertain/risky sites and respects its shared budget. Use `{"embeddings":true,"full":true}` for an embedding-only
rebuild without LSP. With both backends enabled, both are rebuilt. The server does not download models.

`metadata.embeddings` reports model/revision, dimensions, index generation,
valid chunks, pending files and freshness. Source freshness and resolution_mode
retain their existing meanings; partial embedding coverage alone does not imply
mixed graph resolution. Inspect both kinds of freshness. See [EMBEDDINGS.md](EMBEDDINGS.md).

## Progress and response detail retention

For long embedding syncs, send `params._meta.progressToken` outside tool arguments
to receive `notifications/progress`. Notifications are absent without a token.
The worker remains serialized; ping/cancellation continue on the reader thread.
`data.embeddings` carries new/reused chunk counts, and `metadata.operation`
contains final phase/state/timings. CLI status can inspect the same sidecar while
the cache is busy. See [operations](OPERATIONS.md).

Explain trims relationships/docs before identity and retains original relation
counts. Search trims snippets before removing candidates. A minimum summary that
cannot fit is an explicit error, never empty successful evidence of absence.
`metadata.reasons` distinguishes response_budget, result_limit, candidate_limit,
parser_error, source_unverified and embedding_coverage.

## Indexing scheduler statistics

`code_rcl_sync` with `embeddings:true` reads the same project batching/profile
configuration as CLI. No extra tool or implicit indexing/download is added.
Sync data includes logical `embedded_chunks`, `unique_inferences`,
`deduplicated_chunks`, saved-vector `reused_chunks`, actual/padded tokens and
batch fill. Length mode adds scheduler peak/staging counters. Progress-token
notifications and the sidecar carry active-file/batch information during native
inference; ping and cancellation remain available. Switching file/length needs
no rebuild; experimental compact changes pipeline and needs `full:true`.
See [indexing measurements](INDEXING-BENCHMARK.md) for quality gates.

### C/C++

Language filters accept `c`, `cpp`, and the alias `c++`. Symbols include
`concept` and `destructor`. Precise uses installed clangd and requires a compilation
database or shared flags. Header rules and clangd driver permissions come from
project configuration, not arbitrary MCP commands. Missing context is an error
unless heuristic fallback is explicit. Build-context changes invalidate precise
caches, including `no_sync` reads. See [C/C++ setup](C-CPLUSPLUS.md).


Selective LSP verification is opt-in: `sync --precise-selective` (MCP
`precise_selective: true`). See [selective precise mode](PRECISE-SELECTIVE.md) for coverage, backend setup, deadlines and cache recovery.


For a bounded verification pass followed by a focused query:

```json
{"name":"code_rcl_sync","arguments":{"project":"/absolute/project","precise_selective":true}}
```

Pass the inner arguments object to the MCP tool. Only sync/graph/explain/impact/path
accept precise_selective. Sync `full:true` is the force parameter; do not send the
CLI flag name precise_full through MCP. Read the running server's tools/list after
an upgrade. If precise_selective is absent, restart/reconnect the MCP server using
the updated executable. Skill updates alone cannot add a capability to an older
server. The initial instructions, tool descriptions and parameter schemas expose
these semantics directly to agents.
