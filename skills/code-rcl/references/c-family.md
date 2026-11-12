# C/C++ analysis - 0.9.1

Use `c` and `cpp` (`c++` is an alias) for C17/C23 and C++23 source. AST analysis
does not require a compiler. Precise uses a user-installed clangd, discovered on
PATH or through `CODE_RCL_LSP_CLANGD`; the project's compiler may remain GCC,
Clang or MSVC. Do not install tools, run builds, or download dependencies merely
to inspect code.

Precise requires `compile_commands.json` or `compile_flags.txt`. Discovery checks
the root then `build/`; `[c_family] compilation_database_dir` selects an explicit
directory. The JSON database wins over shared flags. A source file must have a
command or applicable flags. Missing/malformed context fails explicitly; fallback
requires `allow_heuristic_fallback` or `--allow-heuristic-fallback` and is partial.

For trusted CMake projects, users can generate the database using Ninja/Makefile
generators and `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`. code-rcl does not invoke
CMake. Preserve the actual project's standard/target/include/macro flags.

`.C` is C++; `.h` follows an override, explicit entry, or its includers.
`[c_family] header_language_overrides` is an ordered list of `{ pattern, language }`
with `c` or `cpp`; the first match wins. A shared C/C++ `.h` needs an explicit
choice for precise. There is one active header representation, not every target
and macro configuration. Inspect `semantic.language_details.build_context`.

`[precise.clangd] query_drivers` defaults to empty. Explicit entries authorize
clangd to execute matching compiler drivers for include discovery; use narrow
paths when needed. System libraries and headers must already be installed.

Build context, header rules, clangd configuration, driver rules and executable
identity changes invalidate cached answers even without source edits. A stale
`no_sync` request must be repaired by sync, with precise enabled when desired.

An LSP external/non-node answer suppresses local guesses. Distinct multiple LSP
targets remain unresolved with a warning. Heuristics do not establish macro
expansion, ADL, dependent template resolution, or runtime dispatch. `concept` is a
type-family node; `destructor` is callable. Declarations and definitions retain
separate identities. Module/header-unit syntax may exceed grammar coverage;
inspect parser diagnostics and do not claim complete C++23 conformance.


In 0.9.1, an uncertain `.h` gets a syntax probe: use C++ only when C has parse
errors and C++ parses cleanly. A clean C parse stays C; if both fail, keep the
previous fallback and report partial analysis. Overrides and established compile/
includer evidence take precedence. `header_language_source=syntax_fallback` is
recorded in symbol build-context details; `header_language_clear` stays false.
Syntax alone never permits precise header queries. Configure an explicit override
or a clear includer/build context when precise verification is required.
Probe results persist by source hash and policy revision; warm sync reuses them.
