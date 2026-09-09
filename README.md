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
code-rcl dump ./path/to/project -f context.md

# Increase per-file size limit to 100 KB
code-rcl dump . --max-size-kb 100 -f full-context.md
```

### 2. Relation-Aware / Targeted Dump
When working with large codebases, dumping everything can exceed token context windows or degrade LLM reasoning. Use `--relation` (`-r`) to extract only the target file/symbol and its connected dependency neighborhood:

```bash
# Dump only files connected to `build_graph` within 2 degrees of relationship
code-rcl dump -r build_graph

# Dump files connected to a specific module/file with a custom depth of 3
code-rcl dump -r src/commands/dump.rs --depth 3 -f dump-feature-context.md

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

### Edge Resolution & Confidence Scoring

Relationships between symbols and files are resolved via multi-tier heuristics:
1. **Import Edges (`confidence = 1.0`):** Language-specific path and module resolution.
2. **Local References (`confidence = 0.95`):** Definitions and references within the same lexical scope or file.
3. **Imported Calls (`confidence = 0.8 - 0.9`):** Calling an identifier explicitly imported from another module.
4. **Project-Wide Unique Match (`confidence = 0.6 - 0.7`):** Unambiguous reference matching a unique exported symbol across the workspace.
5. **Receiver / Method Match (`confidence = 0.4 - 0.5`):** Associated methods matched by signature and receiver heuristics.

Adjust `--min-confidence` (default: `0.4`) to fine-tune graph density.

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
