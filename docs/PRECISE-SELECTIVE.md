# Selective precise verification

By default, normal sync runs Tree-sitter and the heuristic resolver without starting a language server. Project configuration can explicitly opt sync into selective or full mode.
`code-rcl sync --precise-selective` adds bounded LSP verification of ambiguous,
unresolved, weak-evidence and risky references. MCP accepts `precise_selective: true`
on tools whose schema exposes precise options. Full `--precise` remains strict;
the two modes cannot be combined. `--precise-full` forces rechecks in the selected
mode, including bypassing the failure cooldown.

```toml
[precise]
enabled = false
selective = true
selective_budget_secs = 30
timeout_secs = 15

[precise.vue]
# Optional explicit SDK and package directories:
# tsdk = "node_modules/typescript/lib"
# plugin_path = "node_modules/@vue/typescript-plugin"
```

Explicit CLI mode flags override configuration. Existing `enabled = true` continues
to mean full verification. Do not enable both configuration modes. Budget bounds
are 1–3600 seconds. The deadline covers server discovery, startup, indexing and
queries after local reference analysis; cleanup is timed separately and bounded.
Servers are created only when there are uncached candidates. Definition requests
use batches of at most 64. C/C++ share clangd, and compatible JS/TS/React variants
share a TypeScript backend session. Vue has its TypeScript bridge companion.

Selection uses unfiltered heuristic reference analysis, before confidence filters,
edge aggregation and node caps. Local bindings/parameters that do not produce graph
dependencies are skipped. Strong scope/import evidence (at least 0.90) is skipped
only without risks. Ambiguity, overloads, unknown receivers, dynamic dispatch,
generic/template dependence, macro/conditional declarations and unsupported lookup
can require verification. Scores remain heuristics, not probabilities.

## Backends

| Source | Server | Executable override |
|---|---|---|
| Rust | rust-analyzer | CODE_RCL_LSP_RUST |
| Go | gopls | CODE_RCL_LSP_GO |
| Java | jdtls | CODE_RCL_LSP_JAVA |
| Kotlin | configured JetBrains/fwcd backend | CODE_RCL_LSP_KOTLIN |
| Python | pyright-langserver | CODE_RCL_LSP_PYTHON |
| JavaScript/JSX | typescript-language-server | CODE_RCL_LSP_JAVASCRIPT |
| TypeScript/TSX | typescript-language-server | CODE_RCL_LSP_TYPESCRIPT |
| Vue | vue-language-server --stdio plus TypeScript companion | CODE_RCL_LSP_VUE, CODE_RCL_LSP_TYPESCRIPT |
| Svelte | svelteserver --stdio | CODE_RCL_LSP_SVELTE |
| C/C++ | clangd | CODE_RCL_LSP_CLANGD |

Install servers and their runtimes yourself. No packages, models or project
dependencies are downloaded, and no project build is launched by code-rcl. Project
models and server configuration still matter. See [C/C++ setup](C-CPLUSPLUS.md) for
mandatory compilation commands/flags and explicit compiler driver allowlists.
TOML is configuration and is never sent to a language server.

Vue uses the original `.vue` document and the current `tsserver/request` →
`tsserver/response` bridge, not the removed hybrid option. Install TypeScript,
`@vue/typescript-plugin`, `@vue/language-server` and a compatible
`typescript-language-server` advertising `typescript.tsserverRequest`. SDK/plugin
paths are discovered in local and server installations, or configured explicitly.
Unmappable virtual documents are non-node results and never high-confidence guesses.
Svelte also receives original source and source positions in UTF-16 LSP coordinates.

## Coverage and recovery

Read `metadata.precision.languages`: candidates, cache_hits, queried, verified,
unresolved, skipped, failed, pending, server_starts and reason codes. The graph
also embeds `precision`, and its viewer legend summarizes coverage. Reasons for
selection and per-reference completion are persisted independently of graph caps.
`complete: true` in selective mode means the selection policy completed, not that
all references were compiler-verified. An empty LSP answer is checked but unresolved;
any subsequently used local result keeps heuristic evidence. External, non-node
and multiple-location answers suppress unsupported local guesses.

Selective failures return `partial` with recovery hints while preserving successful
answers from other requests/backends and the remaining heuristics. Full precise
failures still require explicit `--allow-heuristic-fallback` to return partial output.
Missing executable/override: install the named server or correct/unset its override.
Missing dependency/project model: prepare that language's SDK/build context yourself.
Unsupported capability/protocol or a crash: check server compatibility and stderr.
Budget exhaustion: remaining references stay pending; rerun to reuse valid answers.
Temporary backend failures cool down for 60 seconds in the same context; source,
context, executable changes and `--precise-full` bypass that cooldown. User
cancellation aborts the operation and cleans up server processes.

Cached answers are per reference. Selective results never mark a whole file checked;
a later full pass still queries its unverified references. Fingerprints cover source
generation, selection policy, executable bytes/path/arguments, project manifests,
backend settings, compilation context and Vue dependencies. Changing them invalidates
only precise answers, preserving symbols, snapshots and embeddings. Old precise
answers lacking coverage metadata are invalidated automatically. Schema 13, graph 3
and envelope 2 remain unchanged. `--no-sync` refuses incompatible precise caches;
recover with an ordinary sync, then optionally run the chosen precise mode.

Static selection cannot discover every incorrect high-confidence guess. Actual
accuracy and latency depend on the corpus and language server; no percentage
speedup or compiler-level completeness is promised. Real server tests are opt-in.
## Reproducing measurements

Run `python tests/support/compare_precise_modes.py target/debug/code-rcl.exe
 tests/fixtures/selective_precise_app --run-lsp --output target/selective-modes.json`
as a single command. It uses isolated caches for each mode, runs cold and warm,
and reports labelled target precision/recall, ambiguity, unresolved references,
queries, actual server starts and wall/phase timings. A failed strict run is
reported as an error. No automatic installation or fallback is used.

Local Windows debug-build sample on 2026-11-05, installed clangd, self-contained
C/C++ fixture (one sample, no claim about general workloads):

| Mode | Cold queries / server starts | Warm queries / server starts | Cold / warm wall time | Labelled precision / recall |
|---|---|---|---|---|
| Heuristic | 0 / 0 | 0 / 0 | 0.448s / 0.328s | 1.0 / 1.0 |
| Selective | 1 / 1 | 0 / 0 | 6.966s / 0.441s | 1.0 / 1.0 |
| Full | 6 / 1 | 0 / 0 | 7.222s / 0.577s | 1.0 / 1.0 |

Each mode matched four positive target labels with no forbidden target. All had
zero ambiguous edges and unresolved graph references on this deliberately small
valid fixture. Startup/indexing dominate cold timings; fewer queries do not
promise a proportional latency gain. C/C++ share one server session. Executable
content identities are cached against path, size and timestamps to avoid repeatedly
hashing a large unchanged server binary; changed project sources still use hashes.
Backend package manifests/entry points also contribute when servers use wrappers.

The before/after labelled heuristic regression harness covered 18 fixtures across
all supported language families: 43/43 positive labels in both builds and no
forbidden labelled edges. This measures labelled cases, not exhaustive precision.
Selection-policy tests additionally cover strong results, unknown targets, Unicode,
LSP overlay independence, dispatch risk and precise-only interrupted-cache recovery.
Mock subprocess tests cover every backend family, React IDs, Vue companion bridge,
missing SDK/plugin, virtual targets, multi-target suppression, errors, batching,
cache transitions, stale context, deadline/cooldown and active MCP cancellation.

Actual server verification in this session used clangd for C17, C23, C++23, build
flag invalidation and the cold/warm comparison above. Rust/Go/Java/Kotlin/Python/
JS/TS/Vue/Svelte server integrations used mocks; their real server tests were not
run. Vue/Svelte runtime-dependent accuracy still requires testing against the
project's installed SDK and server versions.
