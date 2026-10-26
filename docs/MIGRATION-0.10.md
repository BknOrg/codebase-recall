# Migration to 0.10.0

Lexical search is still the default. Nine MCP tool names, JSON envelope version
1, graph payload version 2 and strict precise behavior are unchanged.

- Install the new binary, then explicitly rerun your chosen agent setup command
  to update the entire skill bundle. Reload the MCP client to refresh schemas.
- Semantic search needs an explicitly prepared model. Initial indexing can be
  scheduled with `embeddings sync`; normal semantic/hybrid searches refresh
  pending in-scope embeddings. Standalone source sync does not run inference.
  Search never downloads weights.
- MCP search adds `retrieval`; MCP sync adds `embeddings`. Results retain the
  envelope and add embedding freshness/coverage metadata. Inspect it separately
  from source freshness. A missing model is an error, not a lexical fallback.
- Cache schemas 10/11 upgrade additively to 12, retaining analysis IDs and existing
  vectors. Source snapshots and the derived content index are filled by normal
  source sync without embedding inference or reparsing unchanged files. Schema
  9 and other incompatible layouts are rebuilt as 12. Config, reports and model files are retained.
  Previously downloaded Gemma 2 or safetensors files cannot substitute for the
  six pinned Gemma 1 Q4 ONNX files.
- Standard builds now include the native CPU runtime, but not the model weights.
  Linux standard packages use glibc; musl remains available without embeddings.
  `--no-default-features --features viewer` preserves a lightweight build.
- Changing embedding dimensions or the chunk format requires explicit full
  indexing. 768 dimensions is the default; 256 is an optional storage tradeoff.

See [the usage guide](EMBEDDINGS.md) and [third-party notices](../THIRD_PARTY.md).
No configuration in your global agent directories is updated automatically.

Batching updates retain the full pipeline and existing vectors: `batching="file"`
is the default, and `batching="length"` is an explicit alternative. Switching
between them needs no rebuild. New optional chunk provenance fields deserialize
old cache payloads unchanged. The experimental compact profile uses a distinct
pipeline, requires `sync --full`, and stays restricted to the benchmark harness
until quality gates pass. No AST-cache schema migration is required for batching.

Development builds can share the `0.10.0` version string while containing different
changes. Use the current checkout build or compare executable fingerprints when
validating batching support. Installing the binary/skill bundle globally and
restarting an existing MCP server are separate explicit actions.

Do not use an older executable against a schema-12 cache if you need to retain
its indexes: older binaries may rebuild it because they do not recognize that
schema. Cache migration occurs locally per project, not by changing global agent
configuration. Backfilling source snapshots is resumable and does not change the
embedding pipeline or require `embeddings sync --full`.

Search adds `--lexical`, qualified-name lookup, shared result limits and
`match_counts`. MCP accepts `retrieval: auto`, while its default remains lexical
until the automatic feature pipeline passes its release gates. No `--rerank`
flag is retained. Optional relevance/support fields and automatic
Gemma context indexing are described in [automatic search](AUTOMATIC-SEARCH.md).
