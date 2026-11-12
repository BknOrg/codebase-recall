# Semantic resolution fixture comparison

Measured on 2026-11-05 against repository baseline `2121361` and this change.
Both binaries used the same fixture bytes with `--min-confidence 0 --max-nodes 0`,
without language servers or embedding models. The baseline was built without
optional features; the changed build used all features. Graph resolution does not
require those optional runtimes.

Required labelled expectations improved from **31/33 to 33/33**. The overload
fixture contributes both a call-presence expectation and a target-declaration-line
expectation for the same call. There were **0 forbidden edges across 16 negative
labels** in either build. These labels are a small regression corpus, not an
exhaustive measurement of project-wide precision.

The previous resolver emitted no call edge for the tied same-arity Java overloads.
The new resolver selects the `int` overload for an integer literal, while an
unknown expression produces an explicitly ambiguous guess. Six emitted edges in
the changed fixtures are marked ambiguous, including unknown-receiver cases that
previously appeared more certain. Unresolved reference counts were unavailable
in the previous graph format; no before/after improvement is claimed for them.

| Fixture | Required before | Required after | Forbidden after / labels | Ambiguous after | Unresolved after |
| --- | ---: | ---: | ---: | ---: | ---: |
| `external_recv_app` | 3/3 | 3/3 | 0/3 | 0 | 3 |
| `generic_type_app` | 4/4 | 4/4 | 0/2 | 0 | 11 |
| `local_shadow_app` | 3/3 | 3/3 | 0/1 | 1 | 5 |
| `python_precise_app` | 3/3 | 3/3 | 0/1 | 1 | 0 |
| `resolve_app` | 3/3 | 3/3 | 0/3 | 1 | 3 |
| `rust_precise_app` | 4/4 | 4/4 | 0/1 | 1 | 1 |
| `same_type_name_app` | 4/4 | 4/4 | 0/4 | 1 | 0 |
| `semantic_go_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_java_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_js_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_kotlin_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_overload_app` | 0/2 | 2/2 | 0/1 | 1 | 1 |
| `semantic_svelte_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_ts_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |
| `semantic_vue_app` | 1/1 | 1/1 | 0/0 | 0 | 0 |

The labelled-subset precision ratio is 100% in both builds wherever a required
edge was emitted. Recall is 93.9% before and 100% after across these expectations.
Additional unlabelled edges are not silently counted as correct. Some fixtures
intentionally contain incomplete source and compile-time-invalid fragments to
exercise syntax extraction and uncertainty; these tests do not compile Java,
Kotlin, or the other target languages.

Reproduce using independently built before/after CLI executables:

```sh
python tests/support/compare_resolution.py BEFORE_BINARY AFTER_BINARY --output target/semantic-accuracy.json
```

The JSON includes per-fixture language tags, labelled precision/recall, forbidden
strong edges, ambiguous edges and unresolved-reference counts. Rust, Python,
Java, Kotlin, Go, JavaScript, TypeScript, Vue and Svelte are represented. Tests
also inspect canonical symbol facts, nested ownership, Unicode positions, bounded
alternatives, visibility, structural edges, and incremental/cache behavior; see
[the migration guide](MIGRATION-SEMANTIC-MODEL.md).

## C/C++ addition

The expanded labelled corpus contains 17 fixtures: 39 required expectations and
20 forbidden pairs. Comparing the original HEAD baseline (`2121361`) with the
C/C++ build finds 31/39 required expectations before and 39/39 after, with no
forbidden pairs emitted. The baseline includes neither C nor C++ support; its
Java overload difference also predates this addition. This comparison therefore
is not a measurement of C/C++ changes alone.

| New fixture | Required before → after | Forbidden after | Ambiguous edges | Unresolved refs |
| --- | --- | --- | --- | --- |
| C | 0/3 → 3/3 | 0/2 | 0 | 0 |
| C++ | 0/3 → 3/3 | 0/2 | 1 | 1 |

The six new call expectations measure symbol name/file selection. One C++
expectation deliberately permits an ambiguous overload guess, rather than
asserting a compiler-verified overload target. Coverage includes that guess;
confidence and alternatives distinguish it from stronger evidence. No previously
labelled language loses a required edge. These small fixtures do not establish
project-wide precision, full standard conformance, or exhaustive overload accuracy.

Reproduce with `tests/support/compare_resolution.py BASELINE CURRENT --output
PATH`; output includes per-fixture languages, labelled precision/recall and
ambiguous/unresolved counts. Local results are saved in
`target/c-family-accuracy.json`. Real clangd tests separately exercise C17, C23,
C++23 and flag invalidation without external dependencies; enable them explicitly
with `CODE_RCL_TEST_PRECISE=1` and the ignored `real_clangd` test in `c_family_cli`.
