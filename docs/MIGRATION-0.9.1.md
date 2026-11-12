# Upgrading to code-rcl 0.9.1

This patch fixes `duplicate primary key` in `search_documents` when a C parser
recovers C++ syntax in a header. Revisited AST nodes no longer create duplicate
declarations, bindings or enum members. Identical search documents are counted
once; conflicting payloads fail before committing analysis.

Uncertain `.h` files now try C first and use C++ only when C fails and C++ parses
cleanly. Explicit overrides, compilation settings, C++ header extensions and clear
includer evidence remain authoritative. If both grammars fail, indexing keeps its
previous fallback and reports partial coverage. Syntax inference is recorded as
`syntax_fallback`; it does not establish a precise compilation context.

Run ordinary `code-rcl sync` after upgrading. C/C++ analyzer revision 3 and header
policy revision 2 refresh affected analysis without deleting the cache. `--no-sync`
rejects old analysis until sync. Source snapshots, baselines and project config
are retained. Schema 13, graph 3 and envelope 2 remain unchanged; header provenance
is additive metadata. Embedding/precise invalidation follows normal analysis
refresh. Do not delete `.code-rcl` as a recovery step.

Skill and MCP versions follow package 0.9.1. Rerun setup for your existing agent
and scope after upgrading the executable, then reconnect MCP to load its schema.

```sh
code-rcl --version
code-rcl sync
code-rcl setup --global --target codex
```

No compiler, LSP, model, dependency download or project build is required for the
header crash fix. Precise mode still needs explicit server/build setup. Existing
0.9 and 0.10 migration documents are retained as historical development notes.
