# C17, C23 and C++23

C and C++ use bundled Tree-sitter grammars for source analysis and an optional,
user-installed **clangd** for precise definition lookup. The project's build
compiler can remain GCC, Clang or MSVC. code-rcl does not build your project,
install clangd, fetch headers, or expand macros itself.

## Prepare precise resolution

Install clangd and put it on PATH, or set `CODE_RCL_LSP_CLANGD` to its executable.
Your compiler toolchain must provide the system headers and standard library.

Generate a compilation database with your existing build configuration, for example:

```sh
cmake -S . -B build -G Ninja -DCMAKE_EXPORT_COMPILE_COMMANDS=ON
code-rcl sync --precise --language c,cpp
```

The CMake export option supports Ninja and Makefile generators, rather than the
Visual Studio generator. Configuration can execute a project's CMake scripts;
run it yourself for a trusted project. code-rcl never runs it automatically.

For a small project, `compile_flags.txt` can contain one argument per line:

```text
-std=c++23
-Iinclude
-DAPP_FEATURE=1
```

Use `-std=c17` or `-std=c23` for C projects. Use `compile_commands.json` for mixed
languages or flags that differ between translation units. The database must
contain one active command per source file; select a different database when a
file has entries for multiple targets. Both `arguments` and `command` entries
are read as data and never executed by code-rcl. UTF-8 response files (`@file`)
are read recursively (up to eight levels), included in freshness checks, and
never executed.

## Configuration and headers

```toml
[c_family]
# Relative to the project, or an absolute directory.
compilation_database_dir = "build"
header_language_overrides = [
  { pattern = "include/c_api/**", language = "c" },
  { pattern = "include/**", language = "cpp" },
]

[precise.clangd]
# Optional: explicitly authorize clangd to query these compiler drivers.
# Leave empty when the normal toolchain headers are found without querying.
query_drivers = []
```

Without an explicit directory, lookup checks the project root, then `build/`.
In the selected directory, `compile_commands.json` wins over `compile_flags.txt`.
An invalid explicit directory or malformed database is an error with a recovery
message. No additional arbitrary build directories are searched.

`.c` is C; `.C` is C++ before extension case normalization. C++ extensions include
`.cc`, `.cpp`, `.cxx`, `.c++`, `.hpp`, `.hh`, `.hxx`, `.h++`, `.ipp`, `.tpp`,
`.ixx`, and `.cppm`. The last two are discovered even when module syntax exceeds
the bundled grammar's coverage; parser diagnostics remain visible.

For headers, the first matching override wins, then an explicit compilation
entry, then source includers. A `.h` used only by C or only by C++ follows that
language. Mixed `.h` files use C++ for heuristic extraction with a warning;
unreferenced `.h` files use C with a warning. Precise requires a clear language
and a direct command, shared flags, or a configured translation-unit includer.
Each header has one active representation, rather than one per macro/target.
Translation units open before header queries; the selected context is recorded
in `semantic.language_details.build_context`.

clangd receives the selected database directory and the project's own flags;
code-rcl does not overwrite the standard, target, macros, or include directories.
`query_drivers` is empty by default: enabling entries allows **clangd** to execute
matching compiler drivers to discover their include paths. Prefer narrow paths.

## Evidence, cache and limitations

Source analysis distinguishes declarations/definitions, namespace/type ownership,
constructor/destructor, operators, aliases, concept, fields, enum members and
macros. Locals and parameters remain scoped bindings. The separate C tag namespace,
declaration order and internal linkage participate in lookup. Conditional source,
templates and virtual calls retain uncertainty; function-pointer calls and macro
expansion require compiler evidence. A header declaration and a compatible source
definition remain separate nodes; a unique match through an include can have an
explicit low-confidence `references` link.

Precise fails when context is absent/incomplete, the server fails, or clangd
reports source/build errors. `--allow-heuristic-fallback` explicitly permits a
partial heuristic result. An external or non-node LSP answer suppresses guesses.
Multiple distinct LSP targets remain unresolved, with a warning, instead of
choosing a first target at high confidence.

Build database/flags, header rules, applicable `.clangd` fragments, user clangd
configuration, driver allowlist, and the clangd executable's path/content identity
participate in freshness checks. Executable content captures version changes
without launching a separate version probe. Changes invalidate precise results;
context or header changes also reparse affected C/C++ analysis. `--no-sync`
rejects stale results: run ordinary sync, or sync with `--precise` to refresh LSP
answers. No schema reset or source-snapshot deletion is needed.

C17/C23/C++23 describe the tested language targets, not complete standard
conformance. Tree-sitter and the installed clangd have feature-specific limits.
Modules/header units, preprocessor configurations, compiler extensions, ADL,
dependent templates, namespace alias/using lookup, conversion ranking, and runtime dispatch are not fully
modeled by source heuristics. Unsupported grammar emits diagnostics and partial
analysis. Objective-C, CUDA and multi-configuration indexing are outside this release.

Official references: [clangd compile commands](https://clangd.llvm.org/design/compile-commands),
[system headers](https://clangd.llvm.org/guides/system-headers),
[CMake database export](https://cmake.org/cmake/help/latest/variable/CMAKE_EXPORT_COMPILE_COMMANDS.html),
[Clang language status](https://clang.llvm.org/cxx_status.html).


In 0.9.1, an uncertain `.h` gets a syntax probe: use C++ only when C has parse
errors and C++ parses cleanly. A clean C parse stays C; if both fail, keep the
previous fallback and report partial analysis. Overrides and established compile/
includer evidence take precedence. `header_language_source=syntax_fallback` is
recorded in symbol build-context details; `header_language_clear` stays false.
Syntax alone never permits precise header queries. Configure an explicit override
or a clear includer/build context when precise verification is required.
Probe results persist by source hash and policy revision; warm sync reuses them.
