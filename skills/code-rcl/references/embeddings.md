# Embeddings — 0.9.1

Use lexical search for identifiers, semantic for behavior descriptions, and
hybrid when both are present. CLI examples:

```sh
code-rcl embeddings setup
code-rcl embeddings sync
code-rcl embeddings status --json
code-rcl search "where cancellation is checked" --semantic
code-rcl search "cancel request" --hybrid
```

Setup explicitly downloads a pinned Gemma 1 Q4 model. `setup --model-dir PATH`
verifies an existing pinned ONNX directory. Indexing uses project sync filters
and stores vectors beside AST data in cache.bkndb. Normal semantic/hybrid search
refreshes pending embeddings in its selected scope after source sync; --no-sync
skips both and stays unverified. No search downloads a model. Explicit embeddings
sync remains available; use --full after changing dimensions (768 default, 256 optional).

MCP `code_rcl_search` accepts `retrieval: auto|lexical|semantic|hybrid`. Lexical stays
the default until the automatic pipeline passes release gates. Explicit auto task
queries fail clearly while unavailable; auto identifiers can use lexical lookup.
Use --lexical to force lexical search. There is no --rerank option. Inspect
metadata.retrieval for selected mode, reason, refresh and pipeline availability.
`code_rcl_sync` with `embeddings:true` explicitly updates the index; setup must
already be complete. Add `full:true` for an embedding-only rebuild without LSP;
with `precise:true` as well, full rebuilds both selected backends.
Keep project explicit when the server has no fixed root.

Inspect metadata.embeddings: valid chunks, pending/excluded files, source_verified and
embedding freshness. Source freshness and embedding freshness are different.
Partial results exclude invalidated chunks. --no-sync means cached state was
not verified. A missing model or no valid chunks is an error, including hybrid.
Do not silently substitute lexical output or describe partial coverage as full.

Use source/explain/impact after retrieval. Cosine and RRF scores rank candidates;
neither is a dependency edge or a calibrated probability. Empty retrieval does
not establish that relevant code or a dependency is absent. Match kind/exported
filters apply to both branches. String/grep flags are lexical-only. Hybrid uses
RRF (constant 60), up to 100 candidates per branch; inspect truncation metadata.

Long syncs show phases and a heartbeat on stderr; this is not proof that an
inference batch completed. Use embeddings status --files to distinguish pending
indexing, parser-blocked files and configured exclusions. During a cache lock,
status returns saved coverage with unverified freshness plus operation progress.
First Ctrl+C waits for the native batch/load; second forces exit. Resume with
ordinary embeddings sync. Do not recommend --full solely for cancellation recovery.

Search/report can use languages, paths, exclude_paths and configured categories
(MCP arrays; CLI --language/--path/--exclude-path/--category). Scope is explicit;
do not infer language from the query or automatically exclude all tests.

For explicit indexing, `[embeddings].batching="length"` groups similar-length
inputs across up to eight files. `file` remains the default/rollback path; changing
batching needs no rebuild. Both CLI and MCP use the same service and keep four
automatic CPU threads (`threads=0`) and batch four by default. Compare measured cold-index runs, not no-op syncs.
Check `unique_inferences`, `deduplicated_chunks`, token padding, active files and
staging counters; a heartbeat means alive, not that a batch has completed.

`chunk_profile="full"` preserves existing inputs. `compact` remains restricted
to the experimental harness until retrieval gates pass; changing profile needs
an explicit full rebuild. Experimental `container_summary` matches keep original
source snippets and include `symbol_range`; never quote synthetic input as source.
Do not promise acceleration or exclude tests/generated code without user scope.

Current local gates did not promote either optimization: length retained retrieval
hit rates but missed strict vector equivalence, so file remains default; compact
reduced class/type held-out top-1 and remains harness-only. Do not enable compact
for normal agent work or present a local speed measurement as a universal promise.

CPU budget applies to indexing and search. Auto uses available logical CPUs,
with a one-thread fallback; manual `threads=1..64` overrides it. Status reports
`threads_configured` and `threads_effective`. Changing only the CPU budget
reuses valid vectors; it does not require `--full`. Auto does not guarantee
maximum utilization or faster indexing for every project.
