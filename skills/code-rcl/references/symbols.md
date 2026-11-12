# Symbol semantics - 0.9.1

Read this when interpreting graph evidence, disambiguating declarations, or
recovering after the semantic-model upgrade.

In the viewer, node colors group symbol families; shapes and zoom-dependent
glyphs identify kinds even under subsystem coloring. Relation colors retain
their meaning during selection. Dashed links include ambiguous guesses;
Filters can hide those guesses while preserving resolved links in mixed
aggregates. Select a node for evidence and alternatives. Structural relations
must be requested when generating the graph before their filters are available.

## Interpret declarations and evidence

Envelope version 2 contains graph format 3; cache schema is 13. These versions
are independent of the executable's package version. Inspect the actual response.

`semantic` records canonical kind, qualified name, owner, scope, visibility,
modifiers, name byte range, parameter/return types and structural relations.
`unknown`/`null` means unavailable evidence, not false. Written and inferred types
are distinct facts. Locals/parameters remain lexical bindings; fields, properties,
constructors and enum members have their own symbols. Go defined types (`type`)
are distinct from `type_alias`.

Prefer qualified names or returned declaration IDs over a simple name for an
overload or nested type. IDs include file, byte range and kind, distinguish
same-line declarations, and can change after edits. Never infer line numbers from
the ID suffix. Use `path`, `lines`, and `semantic.name_range` instead.

`resolution` explains each selected call/reference: evidence, reason, confidence,
ambiguity, up to three alternatives and the number omitted. Ambiguous or unknown-
receiver guesses are capped at 0.35. Alternatives are candidates, not extra edges.
Scores are not probabilities. LSP external/non-node answers suppress local guesses.

`resolution_summary` counts reference sites in the requested path/edge-kind scope
before node caps. It is not the deduplicated edge count. Owner-to-type references
with `member_type` evidence preserve impact through field/enum-member types.
Reports expose uncertainty counts; impact/path/explain retain relation evidence.
Digest types expose semantic facts and `members`; file `declarations` hold other
symbols. Output budgets can trim these details before identities/counts: inspect
`metadata.truncated` and `trimmed_details`, then narrow or increase the budget.

## Choose graph scope

For MCP `code_rcl_graph`, use `kinds` to select `calls`, `imports`, `references`,
`contains`, `inherits`, `implements` or `overrides`. Defaults are imports/calls/
contains; structural traversal requires explicit kinds. `min_confidence` defaults
to 0 for MCP graph; 0.5 hides ambiguous guesses. CLI graph defaults to 0.3 unless
project configuration overrides it. Impact/path accept the same edge-kind tokens.

Source-level inheritance is not complete runtime dispatch or generic constraint
solving. Unknown decorators and dynamic behavior are not executed. Read source
and use an installed language server for compiler-dependent conclusions.

## Recover stale caches

Schemas 10–12 migrate transactionally to 13, preserving file identities, available
source snapshots, configuration and historical metadata. Derived semantic/search/
embedding indexes and precise results are invalidated. Run normal source sync;
analyzer revisions can trigger reparse even without source changes. `no_sync`
rejects stale semantic generations. Filtered sync can leave other languages stale.
Refresh all required languages before querying the whole project.

Rebuild embeddings through the existing prepared-model workflow. Neither migration
nor source sync downloads a model. Use the upgraded executable for sync; an older
release may use an incompatible cache layout. Reinstall the bundled skill with the
same setup scope and restart the MCP process after updating the binary.
