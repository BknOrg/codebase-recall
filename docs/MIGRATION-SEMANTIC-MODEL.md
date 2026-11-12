# Semantic model migration

This change introduces cache schema **13**, graph format **3**, and CLI/MCP
envelope **2**. It covers existing source languages; it does not add C/C++.

## Upgrade

Run `code-rcl sync` before consuming cached results. Cache schemas 10–12 migrate
in one transaction, preserving file identities, original source snapshots (where
available), configuration, and historical metadata. Symbols, references, search
documents, purpose contexts and vectors are derived indexes and are invalidated.
Schema 10 source snapshots are populated from source on the next sync.
Interrupted migration rolls back. A conflicting schema-10 snapshot table is an
error, not permission to reset data.

`--no-sync` rejects stale semantic generations and tells you to run source sync.
Filtered sync only refreshes its selected files; remaining stale files still need
sync before a whole-project query. Analyzer revisions trigger reparse even when
source hashes are unchanged. Rebuild embeddings with `code-rcl embeddings sync`
after configuring a model. Migration and ordinary source sync never download one.

## Public output

Read the `data` field inside envelope `schema_version: 2`. Graphs have
`data.version: 3`. Error and budget-limited MCP responses use the same envelope
version. Budget-limited responses may omit semantic details and report trimming in
metadata; declaration identities and relation counts take priority. Regenerate saved graphs and update clients that validate these versions.

Symbol IDs now include file, name, byte range and kind:
`sym:src/a.rs#run@40:85:method`. They distinguish declarations on the same line;
they are not persistent identities across edits. Do not parse line numbers from
IDs. Use `lines`, `path`, or the semantic name range. Qualified names are searchable
and identify candidates; they do not collapse overloads into one declaration.

`kind` uses a shared typed vocabulary. Changes include:

| Previous grouping | More specific kinds |
| --- | --- |
| `type` for aliases | `type_alias` (Go defined types remain `type`) |
| Java record as `class` | `record` |
| Kotlin object as `class` | `object` |
| Constructor as `method` | `constructor` |
| Rust union as `struct` | `union` |
| Const as `variable` | `constant` |
| Fields held only as bindings | `field`, `property` |
| Enum entries omitted | `enum_member` |

Symbols expose `semantic`: kind, role, qualified name, owner, package, name byte
range, scope, visibility, modifiers, declared/inferred types, ordered parameters,
return type, type parameters, extension receiver, structural relations and
language details. Unknown values remain `unknown` or `null`; absent type evidence
does not mean a negative fact. Local variables and parameters remain lexical
bindings rather than filling the graph with local-variable nodes.

Immutable bindings and constant declarations are different concepts. For example,
JavaScript `const` may still hold a mutable object. Visibility and exported status
are also separate; the latter remains a convenience summary for listing filters.

## Resolution evidence and coverage

Call/reference edges expose `resolution` with `evidence`, `reason`, `confidence`,
`ambiguous`, `alternatives` and `omitted_alternatives`. Alternative entries contain
declaration ID, qualified name and path. A tie selects one deterministic primary
guess, lists up to three alternatives, and caps confidence at **0.35**. Alternative
candidates do not create additional edges or increase graph degree.

New configurations default to `min_confidence = 0.3` to include these guesses.
Existing explicit configuration remains authoritative. Use `--min-confidence 0.5`
to hide ambiguous guesses. Scores rank heuristic evidence; they are not calibrated
probabilities. LSP definitions outrank heuristics, and external/non-node LSP
answers suppress invented local targets.

Graph `resolution_summary` counts eligible, resolved, ambiguous, unresolved,
local-only, external/non-node and below-threshold references in the requested
path/edge-kind scope, before node caps. Counts describe reference sites, not the
deduplicated edge count. Type references on fields/enum members also retain an
owner-to-type dependency with `member_type` evidence, so impact can still reach
the owning type. Architecture reports include this summary and an
ambiguous-edge count. Search/digest/explain expose symbol facts; impact/path and
explain relationships preserve resolution evidence. Digest types carry semantic
facts and typed `members`; other declarations appear in each file's `declarations`.
Digest/report totals count all source symbols, including fields and enum members.
The viewer renders ambiguous
links dashed and lists their alternatives in the node panel.

`inherits`, `implements` and `overrides` are structural edge kinds. Select them
explicitly for traversal, for example `--kinds calls,inherits,overrides`.
The existing default graph selection still includes `implements`; it does not
automatically include the new inheritance/override edges.

## Accuracy boundaries

The static resolver uses declared scope, imports, qualified ownership, available
argument types and arity (including default/variadic parameters). Known decorator
syntax is recognized; arbitrary decorators, metaclasses, monkey-patching and
runtime-generated exports are not executed. Structural edges require identifiable
source declarations; incomplete type information does not prove an override.
Full generic constraint solving, runtime dispatch and compiler-specific semantic
analysis remain language-server responsibilities. A syntactically accepted file
is not proof of compiler-equivalent interpretation.

Fixtures in `tests/fixtures/semantic_symbols` label expected symbol facts across
all language families. `tests/semantic_model.rs` checks overload evidence,
ambiguity/filtering, consumers, structural edges and incremental/full equivalence.
`tests/resolve_accuracy.rs` retains independently labelled positive/negative call
edges; its negative coverage is not an exhaustive ground truth for a project.
