# Selective verification

Use this reference when choosing heuristic, selective or full LSP verification.

| Mode | CLI sync | MCP sync arguments | Failure / coverage |
|---|---|---|---|
| Heuristic (default) | `code-rcl sync` | `{}` | No new LSP session unless project config opts in; valid cached answers remain usable |
| Selective | `code-rcl sync --precise-selective` | `{"precise_selective":true}` | Uncertain/risky sites only; partial on errors/budget exhaustion |
| Full | `code-rcl sync --precise` | `{"precise":true}` | All pending sites; strict unless explicit fallback |

Use an explicit absolute `project` in MCP requests without a configured root.
Do not combine precise and precise_selective or ask to verify with no_sync=true.
Normal queries need not launch LSP. Choose extra verification when the task needs
it; do not claim Tree-sitter confidence is compiler proof.

## Request support

CLI: sync, graph, serve, explain, impact and path accept --precise-selective.
MCP: code_rcl_sync, code_rcl_graph, code_rcl_explain, code_rcl_impact and
code_rcl_path accept precise_selective. Search/report/digest consume stored
answers and coverage; sync first if extra verification is needed. Their schemas
do not accept a precision flag. Read tools/list before calling an older server;
package version alone does not establish support.

```json
{"project":"/absolute/project","precise_selective":true}
```

After sync, use a focused explain/impact/path/report request. Example explain:

```json
{"project":"/absolute/project","symbol":"QualifiedName","precise_selective":true}
```

Symbol, path, node caps, confidence filters and output budgets do not bound LSP
reference selection. Selection runs before graph presentation. For explicitly
restricted language analysis, use sync's languages array. TOML remains config.

## Configure and recheck

```toml
[precise]
enabled = false
selective = true
selective_budget_secs = 30
timeout_secs = 15
```

Without config, both mode defaults are false. Config supplies defaults to CLI
sync/graph/serve and MCP sync/graph. Explicit mode flags/fields override config;
MCP precise:false disables a configured mode for that request. Existing enabled=true
means full mode. Never set enabled and selective true together.

Startup, indexing and queries share the selective budget; per-request timeout
cannot extend it. Force CLI with --precise-selective --precise-full, or MCP sync
with precise_selective:true, full:true. Full means force in the chosen mode;
it does not change selective into full verification, reparse all source, or fetch
a model. Valid per-reference cache is reused without queries on warm runs.

## Interpret evidence

Read metadata.precision.languages: candidates, cache_hits, queried, verified,
unresolved, skipped, failed, pending, server_starts, reasons, selection_reasons.
Selective complete:true means its selection policy completed, not that every site
was verified. An empty LSP answer is checked/unresolved; any local fallback still
carries heuristic evidence. External/non-node and multi-target answers suppress
unsupported guesses. Skipped bindings do not produce graph dependency edges.

resolution_mode describes actual evidence: heuristic, mixed, precise_cached.
A successful selective invocation is usually mixed. Partial results are usable
with their stated limits, not proof of complete dependency coverage. Inspect
relation.resolution.evidence, ambiguity and alternatives; a solid viewer edge
alone does not prove LSP verification. The viewer legend also summarizes coverage.

## Recover without retry loops

Missing LSPs/dependencies are reported, never installed automatically. Correct the
named CODE_RCL_LSP_* override or prepare the server/runtime/build context yourself.
Temporary failures cool down 60 seconds for the same context. Source/context/server
changes or a deliberate force recheck bypass cooldown. Retry ordinary selective
sync after fixing setup to preserve cache reuse; do not repeatedly force checks.
Use normal sync to repair stale precise cache rejected by --no-sync.

C/C++ require clangd plus compile_commands.json or compile_flags.txt; compiler
standards/flags come from the build, not injected defaults. See [C/C++](c-family.md).
Vue needs vue-language-server, TypeScript SDK, @vue/typescript-plugin and a
TypeScript companion supporting typescript.tsserverRequest. Optional [precise.vue]
tsdk and plugin_path point to SDK/package directories. Svelte uses svelteserver
--stdio. Overrides are CODE_RCL_LSP_VUE, CODE_RCL_LSP_SVELTE,
CODE_RCL_LSP_TYPESCRIPT and CODE_RCL_LSP_CLANGD. Original source coordinates remain
in use; unmappable virtual targets are non-node. Real server accuracy depends on
project setup; fewer queries do not promise proportional speed gains.
