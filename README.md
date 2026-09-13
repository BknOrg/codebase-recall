# codebase-recall

> **High-performance CLI codebase context dumper for LLMs and AST-based code relation grapher.**

`codebase-recall` (`code-rcl`) is a command-line tool built with Rust 🦀 designed for two primary developer workflows:

1. **Context Bundling for LLMs:** Scan your directory structure and bundle your codebase—or a focused, dependency-aware slice of it—into a clean, well-structured Markdown document ready for LLMs (ChatGPT, Claude, Gemini, DeepSeek).
2. **Code Relation Graph & Architecture Visualization:** Parse ASTs across multiple languages (Rust, JS/TS, Python, Java, Kotlin, Vue, Svelte) to discover definitions, imports, and cross-file calls. Visualize interactions in real-time in an interactive browser UI or export to self-contained HTML, Graphviz DOT, or JSON.

---

## Key Features

### 📦 Codebase Context Dumper (`dump`)

- **Full & Targeted Dumps:**
  - **Full Dump:** Pack the entire repository with directory trees and filtered source contents.
  - **Targeted / Relation-Aware Dump (`-r / --relation`):** Provide a target symbol or file; `code-rcl` traverses the dependency graph up to `--depth <N>` and dumps *only* connected and relevant files—saving prompt tokens and eliminating hallucination noise.
- **Smart Filtering:**
  - Automatically respects `.gitignore` rules and ignores hidden directories (`.git/`, `.cache/`, etc.).
  - Skips binary files, media assets (`.png`, `.webp`, `.pdf`, etc.), lockfiles (`Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `bun.lockb`, etc.), minified bundles (`.min.js`, `.chunk.js`), and sensitive environment files (`.env*`).
- **Safe Dynamic Markdown Fencing:** Detects the maximum backtick streak in file contents and dynamically expands markdown fences (e.g. ` ```` ` or ` ````` `) to prevent nested markdown from breaking document formatting.
- **Size Limits & Path Normalization:** Configurable per-file size limit (default: 50 KB) and automated path separator normalization across OS platforms.

### 🕸️ Code Relation Graph (`graph` & `serve`)

- **Multi-Language AST Parsing:** Powered by tree-sitter for **Rust**, **JavaScript/JSX**, **TypeScript/TSX**, **Python**, **Java**, **Kotlin**, and Single-File Components (**Vue**, **Svelte**).
- **Multi-Tier Resolution:** Tracks symbols (functions, structs, classes, enums, methods), imports, and call references across files with confidence scoring.
- **Incremental SQLite Caching:** Stores file hashes (Blake3) and AST entities in `.code-rcl/cache.db`. Re-runs only parse files modified since the last sync.
- **Interactive Browser Viewer (`serve`):**
  - Instant local visualization powered by vendored d3-force simulation (smooth zoom, pan, and drag).
  - Toggle edge kinds (`imports`, `calls`, `references`).
  - Interactive search filtering, node isolation, cluster expansion/collapsing, and press-and-hold node spotlighting.
  - Interactive BFS depth adjuster directly inside the web UI.
  - **Zero Background Footprint:** Uses a lightweight EventSource heartbeat; the local server automatically terminates when you close your browser tab.
- **Export Formats (`graph`):** Standalone zero-dependency HTML, Graphviz `.dot`, and structured JSON (`version: 1`).

---

## Installation

### Prebuilt Binaries

#### macOS / Linux (Shell Script)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.sh | sh
```

#### Windows (PowerShell)

```powershell
irm https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.ps1 | iex
```

### Install via Cargo (crates.io)

Ensure you have Rust and Cargo installed:

```bash
cargo install codebase-recall
```

### Build from Source

```bash
git clone https://github.com/BknOrg/codebase-recall.git
cd codebase-recall
cargo build --release
cargo install --path .
```

The compiled binary is available as `code-rcl`.

---

## Subcommands Overview

`code-rcl` is organized into subcommands:

| Command | Description |
| :--- | :--- |
| `code-rcl dump` | Bundle the codebase (or a symbol's neighborhood) into a single Markdown file for LLM context. |
| `code-rcl init` | Initialize `.code-rcl/` (SQLite cache DB + `config.toml`) and ensure it is in `.gitignore`. |
| `code-rcl sync` | Incrementally parse changed source files into the graph cache. |
| `code-rcl graph` | Auto-sync, then export the relation graph to file(s) (`html`, `json`, `dot`). |
| `code-rcl serve` | Auto-sync, then host an interactive relation graph in the browser; exits when the tab closes. |

---

## Usage — `dump` (LLM Context Generator)

### 1. Standard Full Dump

Scan the repository and produce a comprehensive `codebase-context.md`:

```bash
# Scan current directory -> codebase-context.md
code-rcl dump

# Scan specific project directory and specify output filename
code-rcl dump ./path/to/project -f context

# Increase per-file size limit to 100 KB
code-rcl dump . --max-size-kb 100 -f full-context
```

### 2. Relation-Aware / Targeted Dump

When working with large codebases, dumping everything can exceed token context windows or degrade LLM reasoning. Use `--relation` (`-r`) to extract only the target file/symbol and its connected dependency neighborhood:

```bash
# Dump only files connected to `build_graph` within 2 degrees of relationship
code-rcl dump -r build_graph

# Dump files connected to a specific module/file with a custom depth of 3
code-rcl dump -r src/commands/dump.rs --depth 3 -f dump-feature-context

# Skip auto-sync if cache is already fresh
code-rcl dump -r App --no-sync
```

### `dump` Options

| Flag / Option | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `[PATH]` | - | `.` | Target project directory path to scan |
| `-f, --file <PATH>` | `-f` | `codebase-context.md` | Path or name of the output Markdown file |
| `--max-size-kb <N>` | - | `50` | Maximum file size (in KB) for content extraction |
| `-r, --relation <NAME>` | `-r` | `None` | Restrict dump to the target symbol/file and its connected dependency graph |
| `--depth <N>` | - | `2` | BFS traversal depth when using `--relation` |
| `--no-sync` | - | `false` | Do not re-sync graph cache before building targeted relation dump |

### Output Markdown Structure

````markdown
> **Targeted dump**: focused on `build_graph` with depth 2. 

# Directory Tree

```text
├── Cargo.toml
└── src
    ├── cli.rs
    ├── commands
    │   └── graph.rs
    └── main.rs
```

---

# Source Files

## File: `src/commands/graph.rs`

```rs
// Extracted source code with dynamic fence backtick handling...
```
````

---

## Usage — Code Relation Graph (`init`, `sync`, `graph`, `serve`)

### Quick Setup

```bash
# 1. Initialize .code-rcl/ in your project (creates cache.db and config.toml)
code-rcl init

# 2. Parse source files into cache (Blake3-based incremental indexing)
code-rcl sync
```

### `code-rcl graph` (File Output)

Generates static graph visualizations in standalone HTML, DOT, or JSON format:

```bash
# Default: auto-syncs and outputs .code-rcl/code-graph.html
code-rcl graph

# Generate HTML, JSON, and DOT files simultaneously
code-rcl graph --format html,json,dot -o build/graph

# File-level dependency graph only (imports between files)
code-rcl graph --scope file --min-confidence 0.7

# Focus on a specific symbol and its immediate neighborhood
code-rcl graph --focus App --depth 2

# Limit graph nodes to prevent cluttered views
code-rcl graph --max-nodes 2000
```

#### `graph` Options

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--project <PATH>` | `.` | Project directory (cache stored in `<project>/.code-rcl/`) |
| `--format <LIST>` | `html` | Comma-separated output formats: `html`, `json`, `dot` |
| `-o, --output <PATH>` | `.code-rcl/code-graph.<ext>` | Output file or path stem when requesting multiple formats |
| `--scope <MODE>` | `both` | Graph scope: `file` (imports only), `symbol`, or `both` (layered) |
| `--kinds <LIST>` | `imports,calls,contains` | Edge kinds to include (`imports`, `calls`, `contains`, `references`) |
| `--path <GLOB>` | `None` | Filter source files by glob pattern |
| `--focus <NAME>` | `None` | Restrict graph to the neighborhood of a symbol or file |
| `--depth <N>` | `2` | BFS traversal depth around `--focus` |
| `--min-confidence <F>` | `0.4` | Filter out resolved edges below this confidence score `[0.0 - 1.0]` |
| `--include-external` | `false` | Include external dependencies (npm, PyPI, crates.io packages) |
| `--max-nodes <N>` | `4000` | Node cap; drops lowest-degree symbols if exceeded (`0` to disable) |
| `--no-sync` | `false` | Render directly from cache without checking/re-parsing modified files |
| `--precise` | `false` | Resolve calls through real language servers ([see below](#compiler-grade-accuracy---precise)) |

---

### `code-rcl serve` (Interactive Browser Viewer)

`code-rcl serve` resolves the graph and hosts an interactive UI on a local HTTP server (`127.0.0.1`), automatically opening your default browser.

```bash
# Build, sync, and launch the interactive viewer on a free port
code-rcl serve

# Pin port and do not launch browser automatically
code-rcl serve --port 8080 --no-open

# Apply filters directly when serving
code-rcl serve --scope symbol --focus handle_request --depth 3
```

#### How `serve` Works

- **Self-Terminating / Zero Background Leak:** An active browser tab maintains an `EventSource` connection (`/live`). Closing the browser tab drops the connection, causing the server to cleanly exit within 2 seconds.
- **Ctrl-C:** Instantly halts the server.
- **No External Network Dependencies:** D3.js and frontend styles are bundled inside the binary; no external CDNs or network connections are made.

#### Browser UI Features

- **Force Simulation:** Real-time physics layout with smooth zooming, panning, and node dragging.
- **Dynamic Filtering:** Checkbox controls for `imports`, `calls`, and `references`.
- **Search & Isolate:** Real-time search bar with instant node filtering and single-click node isolation.
- **Live Depth Controls:** Adjust BFS depth (`+` / `-`) directly from the browser navigation bar.
- **Node Spotlighting:** Long-press any node to spotlight its immediate connections while dimming the rest of the graph.

---

## Language Support

| Language | Extensions | Parser Engine | Features Detected |
| :--- | :--- | :--- | :--- |
| **Rust** | `.rs` | Tree-sitter | Functions, structs, enums, traits, methods, macros, `use` imports, calls |
| **JavaScript** | `.js`, `.mjs`, `.cjs` | Tree-sitter | Functions, classes, methods, ESM/CJS imports, calls |
| **JSX** | `.jsx` | Tree-sitter | Components, functions, hooks, imports, JSX references |
| **TypeScript** | `.ts`, `.mts`, `.cts` | Tree-sitter | Interfaces, types, classes, functions, modules, imports |
| **TSX** | `.tsx` | Tree-sitter | TS types, React components, hooks, imports, calls |
| **Python** | `.py`, `.pyi` | Tree-sitter | Functions, classes, methods, `import` / `from ... import`, calls |
| **Java** | `.java` | Tree-sitter | Classes, interfaces, enums, records, methods, `import` (incl. `static` / `.*`), calls |
| **Kotlin** | `.kt`, `.kts` | Tree-sitter | Classes, objects, top-level & member functions (incl. `@Composable`), properties, `import` (incl. `as` / `.*`), calls |
| **Vue** | `.vue` | SFC Extractor + TS/JS | `<script>` & `<script setup>` symbols, imports, components |
| **Svelte** | `.svelte` | SFC Extractor + TS/JS | `<script>` symbols, imports, reactive calls |

## Resolution Semantics, Accuracy & Limitations

`code-rcl` implements a tiered reference resolver. By default, it runs a fast AST-based heuristic engine that requires no compilation or external toolchain. For exact resolution, `--precise` integrates Language Server Protocol (LSP) backends to compute compiler-grade definitions.

### Resolution Pipeline (L0 – L4)

References are resolved through a 5-tier pipeline ordered by confidence:

1. **L0 — Compiler Backend (`--precise`):** Queries language servers (`rust-analyzer`, `pyright`, `typescript-language-server`, `jdtls`, etc.) via JSON-RPC (`confidence = 1.0`). References resolving to external packages or standard libraries produce no edge, preventing false cross-file links.
2. **L1 — Lexical Scope:** Resolves same-file definitions during the AST walk (`confidence = 0.95`). Identifiers bound to local variables, parameters, or closures are tagged `local_only = true` and quarantined from cross-file matching.
3. **L2 — Explicit Imports & Namespaces:** Resolves qualified paths (`module::func()`) and explicit symbol imports (`use crate::worker::Retry`) (`confidence = 0.90`).
4. **L3 — Receiver Type Deduction:** Resolves calls on `self`, `this`, `Type::method`, and struct field access chains (`self.worker.run()`) against the project's indexed type definitions (`confidence = 0.80 - 0.85`).
5. **L4 — Scored Disambiguation:** Fallback when the receiver type is unannotated or absent from local definitions. Candidates sharing the callee name are scored by import reachability (+3), export visibility (+2), parameter arity match (+1 to +2), and directory proximity (+1). To prevent false positives, an edge is emitted only if the score margin between the top candidate and runner-up is at least 2 (`margin >= 2`). Exact ties emit no edge.

```mermaid
graph TD
    Ref[Call Site Reference] --> L0{L0: Language Server?}
    L0 -->|Hit| Res0[Target Symbol (1.00)]
    L0 -->|External / Nonode| Drop[Drop Edge]
    L0 -->|Disabled / Unresolved| L1{L1: Lexical Scope?}
    
    L1 -->|local_only| LocalDrop[Local Binding (No Edge)]
    L1 -->|Same-File Symbol| Res1[Local Symbol (0.95)]
    L1 -->|Unresolved| L2{L2: Explicit Import?}
    
    L2 -->|Named Import / Module Prefix| Res2[Import Target (0.90)]
    L2 -->|Unresolved| L3{L3: Receiver Type?}
    
    L3 -->|Field / Param Type Match| Res3[Type Method (0.80 - 0.85)]
    L3 -->|Unresolved| L4{L4: Scored Disambiguation}
    
    L4 -->|Margin >= 2| Res4[Scored Winner (0.40 - 0.70)]
    L4 -->|Margin < 2| NoEdge[Ambiguous: No Edge]
```

### Empirical Verification & Accuracy Benchmarks

Resolver behavior is verified by integration test suites in `tests/resolve_accuracy.rs`. Every fixture is an executable program instrumented to verify runtime execution traces (`cargo run`), paired with database assertions on cache tables (`refs.precise_status`, `refs.local_only`):

| Fixture | Target Scenario | Heuristic (Recall / Prec.) | Precise (Recall / Prec.) | Resolution Behavior |
| :--- | :--- | :---: | :---: | :--- |
| `resolve_app` | Cross-file method & bare module path | **100%** / **100%** | *N/A* | Resolved via L2 imports and L3 receiver fields. |
| `local_shadow_app` | Local closure shadowing same-named function | **100%** / **100%** | *N/A* | L1 marks closure call `local_only`, avoiding false cross-file edge. |
| `rust_precise_app` | Inferred constructor return (`let h = Real::new()`) | 80% / 80% | **100%** / **100%** | Heuristics resolve to imported decoy; `--precise` infers return type. |
| `python_precise_app` | Dynamic method polymorphism | 75% / 75% | **100%** / **100%** | Heuristic misses unannotated instance; Pyright resolves runtime class. |
| `generic_type_app` | Standard library `Vec<T>::push` vs custom `push` | 100% / 67% | **100%** / **100%** | Heuristics match single workspace `push`; `--precise` marks `Vec` call external. |

### Known Limitations

- **Standard Library Method Collisions (Heuristic Mode):** Heuristic mode indexes only files within the workspace. When calling common methods (`push`, `len`, `get`) on standard library types (`Vec`, `HashMap`, `Option`), L3 cannot deduce external receiver types and delegates to L4. If only one method with that name exists in the workspace, L4 scores it as an uncontested candidate. Use `--precise` to suppress external library calls.
- **Generic Type AST Normalization:** `impl<T> Buffer<T>` is indexed with its generic parameters. When called from a site typed as `Buffer<String>`, string-equality checks in L3 may fail to pair the receiver, deferring resolution to L4.
- **Dynamic Language Metaprogramming:** Dynamic dispatch (`getattr`, monkey patching, runtime decorators, eval) cannot be resolved statically from AST alone.
- **Build System Prerequisites for `--precise`:** Compiler-grade backends require valid project manifests (`Cargo.toml`, `tsconfig.json`, `pom.xml`, `build.gradle`). Without a project model, language servers degrade to syntax-only inspection.

---

## Compiler-Grade Accuracy (`--precise`)

When exact call resolution is critical, `--precise` queries the real language server for each language where a name is defined, and stores that answer as the top resolution layer (L0):

```bash
# Resolve through installed language servers
code-rcl sync --precise

# Only one language, with custom timeout
code-rcl sync --precise --language rust --precise-timeout 30

# Re-ask about every file (edits can change how other files resolve)
code-rcl sync --precise --precise-full

# Export graph with compiler precision
code-rcl graph --precise
```

### Supported Language Servers

Nothing is bundled. Install the servers for your languages; a missing server is reported with its install command and that language simply retains its heuristic edges.

| Language | Server | Install Command | Override Variable |
| :--- | :--- | :--- | :--- |
| **Rust** | `rust-analyzer` | `rustup component add rust-analyzer` | `CODE_RCL_LSP_RUST` |
| **Python** | `pyright-langserver` | `npm install -g pyright` | `CODE_RCL_LSP_PYTHON` |
| **TypeScript** | `typescript-language-server` | `npm install -g typescript-language-server typescript` | `CODE_RCL_LSP_TYPESCRIPT` |
| **JavaScript** | `typescript-language-server` | `npm install -g typescript-language-server typescript` | `CODE_RCL_LSP_JAVASCRIPT` |
| **Java** | `jdtls` (Eclipse JDT LS) | [eclipse.jdt.ls releases](https://github.com/eclipse-jdtls/eclipse.jdt.ls) (needs JDK 17+) | `CODE_RCL_LSP_JAVA` |
| **Kotlin** | `kotlin-language-server` | [kotlin-language-server releases](https://github.com/fwcd/kotlin-language-server/releases) | `CODE_RCL_LSP_KOTLIN` |

#### `sync --precise` Options

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--precise` | `false` | Resolve references through the real language servers |
| `--precise-full` | `false` | Re-ask about every file, not just the ones with no answer yet |
| `--precise-timeout <SECONDS>` | `15` | Budget for a single language-server answer |

---

## JSON Schema (`--format json`, `version: 1`)

When exporting with `code-rcl graph --format json`, the output conforms to this structure:

```jsonc
{
  "version": 1,
  "root": "/path/to/project",
  "generated_at": 1730000000,
  "nodes": [
    {
      "id": "file:src/main.rs",
      "kind": "file",
      "label": "src/main.rs",
      "path": "src/main.rs",
      "language": "rust",
      "exported": true
    },
    {
      "id": "sym:src/main.rs#main@10",
      "kind": "function",
      "label": "main",
      "path": "src/main.rs",
      "language": "rust",
      "exported": false
    }
  ],
  "edges": [
    {
      "source": "file:src/main.rs",
      "target": "file:src/cli.rs",
      "kind": "imports",
      "confidence": 1.0
    },
    {
      "source": "sym:src/main.rs#main@10",
      "target": "sym:src/commands/dump.rs#run@10",
      "kind": "calls",
      "confidence": 0.85
    }
  ]
}
```

---

## Configuration (`config.toml`)

Running `code-rcl init` creates `.code-rcl/config.toml` in your project root:

```toml
# code-rcl project configuration
schema_version = 1

[sync]
# Skip source files larger than this many KB
max_file_kb = 512
# Languages to analyze
languages = ["rust", "javascript", "typescript", "python"]

[graph]
# Drop resolved edges below this confidence
min_confidence = 0.4
# Include edges to external (npm / pypi / crate) modules
include_external = false
```

---

## License

This project is licensed under the [MIT License](LICENSE).
