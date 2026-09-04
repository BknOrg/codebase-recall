# codebase-recall

> **CLI codebase context dumper for LLMs and fast project recall.**

`codebase-recall` is a command-line interface built with Rust 🦀 that scans your directory structure and bundles your entire codebase into a clean, well-structured Markdown file—ready to be used as context for Large Language Models (LLMs) such as ChatGPT, Claude, or Gemini.

---

## Key Features

- **Automated Directory Tree:** Generates an instant visual hierarchy of your project's folders and files.
- **Smart Filtering:**
  - Respects `.gitignore` rules and ignores hidden files.
  - Skips binary files, media assets (`.png`, `.jpg`, `.pdf`, etc.), lockfiles (`Cargo.lock`, `package-lock.json`, etc.), and sensitive configuration files (`.env`).
- **Safe Markdown Fencing:** Dynamically adjusts the number of surrounding code block backticks (`` ` ``) to prevent nested Markdown files from breaking the document layout.
- **Max File Size Limit:** Prevents oversized files from inflating LLM token counts (default: 50 KB).
- **Clean & Portable Output:** Normalizes Windows path separators to Unix format.
- **Code Relation Graph:** Parses Rust, JavaScript/TypeScript, and Python with tree-sitter
  ASTs and renders how files, functions, and variables relate. View it live in the browser
  with `code-rcl serve` (a throwaway local server that quits the moment you close the tab),
  or write it to a self-contained interactive HTML page, a Graphviz `.dot` file, or a JSON
  graph. The layout uses a d3-force simulation (Barnes–Hut charge, smooth zoom/drag);
  press-and-hold any node to spotlight it — everything not directly connected fades out.
  Backed by an incremental SQLite cache so re-runs only re-parse what changed.

---

## Installation



### Install prebuilt binaries via shell script (MacOs/Linux)


```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.sh | sh
```

### Install prebuilt binaries via powershell script (Windows)

```sh
irm https://github.com/BknOrg/codebase-recall/releases/latest/download/codebase-recall-installer.ps1 | iex
```

### or install with cargo

Ensure you have **Rust & Cargo** installed on your system.

#### Install from crates.io
```bash
cargo install codebase-recall
```

### Or build from source

1. Clone the repository:
   ```bash
   git clone git@github.com:BknOrg/codebase-recall.git
   cd codebase-recall
   ```

2. Build in release mode:
   ```bash
   cargo build --release
   ```

3. (Optional) Install locally to make the binary executable system-wide:
   ```bash
   cargo install --path .
   ```



# Commands

`code-rcl` is organized into subcommands. **Note:** as of `0.4.0` the Markdown dump lives
under `code-rcl dump` — running `code-rcl` with no subcommand now prints help.

| Command | Purpose |
| :--- | :--- |
| `code-rcl dump`  | Bundle the codebase into a single Markdown context file (the original behavior). |
| `code-rcl init`  | Create `.code-rcl/` (graph cache DB + `config.toml`) in the target project and add it to `.gitignore`. |
| `code-rcl sync`  | Parse changed source files into the graph cache (hash-based incremental). |
| `code-rcl graph` | Auto-sync, then render the relation graph to a file (HTML / DOT / JSON). |
| `code-rcl serve` | Auto-sync, then serve the relation graph in the browser; the server exits when you close the tab. |

---

## Usage — `dump`

Run inside your target project directory (or pass a path).

```bash
# Scan the current directory -> codebase-context.md
code-rcl dump

# Custom target directory and output file
code-rcl dump ./path/to/project -f project-summary.md

# Raise the per-file size limit to 100 KB
code-rcl dump . --max-size-kb 100 -f context.md
```

| Argument | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `[PATH]` | - | `.` | Target directory path to scan |
| `--file` | `-f` | `codebase-context.md` | Name or path of the output Markdown file |
| `--max-size-kb` | - | `50` | Maximum file size (in KB) for content extraction |

---

## Usage — Code Relation Graph

```bash
# One-time: create .code-rcl/ in the project
code-rcl init

# Parse the codebase into the cache (incremental on later runs)
code-rcl sync

# Render the graph — auto-syncs first, writes .code-rcl/code-graph.html
code-rcl graph

# All three formats at once, to a chosen path stem
code-rcl graph --format html,json,dot -o build/graph

# Just the file-level dependency graph, high-precision edges only
code-rcl graph --scope file --min-confidence 0.7

# Zoom in on one symbol and its neighborhood
code-rcl graph --focus build --depth 2
```

### `graph` options

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--project <PATH>` | `.` | Project directory (cache lives at `<project>/.code-rcl/`) |
| `--format <LIST>` | `html` | Comma-separated: `html`, `json`, `dot` |
| `-o, --output <PATH>` | `.code-rcl/code-graph.<ext>` | Output file, or a path stem when multiple formats are requested |
| `--scope <MODE>` | `both` | `file` (imports only), `symbol`, or `both` (layered) |
| `--kinds <LIST>` | `imports,calls,references,contains` | Edge kinds to include |
| `--path <GLOB>` | - | Restrict to files matching a glob |
| `--focus <NAME>` | - | Keep only the neighborhood of this symbol/file |
| `--depth <N>` | `2` | BFS depth around `--focus` |
| `--min-confidence <F>` | `0.4` | Drop resolved edges below this score |
| `--include-external` | off | Show edges to npm / pypi / crate dependencies |
| `--no-sync` | off | Render straight from the cache without re-parsing |

`init` takes `--project` and `--force`; `sync` takes `--project`, `--max-file-kb` (default
`512`), and `--language rust,js,py` to restrict languages.

### Usage — `serve`

`code-rcl serve` builds the same graph but, instead of writing a file, hosts it on a
local HTTP server bound to `127.0.0.1` and opens your browser. It is designed to leave
nothing running in the background:

- The page holds one `EventSource` connection open. When you **close the tab** (or the
  window), that connection drops and the server exits within a few seconds — an open
  connection also keeps it alive while the tab is merely backgrounded.
- **Ctrl-C** in the terminal stops it immediately.
- If no browser ever connects, it gives up after ~90 seconds.

```bash
# Build + serve + open a browser on a free port
code-rcl serve

# Pin the port and don't open a browser (print the URL only)
code-rcl serve --port 742 --no-open

# Same graph filters as `graph`
code-rcl serve --scope file --focus build --depth 2
```

`serve` accepts every `GraphQuery` flag from the table above (`--project`, `--scope`,
`--kinds`, `--path`, `--focus`, `--depth`, `--min-confidence`, `--include-external`,
`--no-sync`) plus:

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--port <N>` | `0` | Port on `127.0.0.1`; `0` picks a free one |
| `--no-open` | off | Don't launch a browser; just print the URL |

The d3 library is vendored into the binary (d3 v7, from `https://cdn.jsdelivr.net/npm/d3`)
and served locally — `serve` makes no outbound network requests, and neither does the
HTML written by `graph --format html`.

### How relations are resolved

Each file is parsed with a tree-sitter grammar (`rust`, `javascript`, `typescript`,
`python`). A first pass records **symbols** (functions, methods, classes/structs/enums,
type aliases, module-level variables), **imports**, and **references**. A second pass
resolves them across files:

- **Import edges** map each `use` / `import` / `require` / `from … import` (and Rust
  `mod foo;`) to a target file via language-specific path rules.
- **Call / reference edges** resolve a name to (1) a same-file definition, (2) an
  imported binding, (3) a unique project-wide export, or (4) a unique same-language
  definition — each tier carrying a lower `confidence`. Receiver-qualified calls only
  match methods/associated functions.

Every edge has a `confidence` in `[0,1]`; raise `--min-confidence` for a cleaner graph.

### Known limitations

- Name-based resolution with no type inference — a method call may link to a
  same-named method in the wrong type. Mitigated by the confidence score.
- Misses dynamic/reflective dispatch, most Rust macro bodies, and Python decorators
  that rewrite dispatch.
- Module resolution is heuristic: no tsconfig `paths`, Cargo workspaces, or
  `pyproject`/`sys.path` maps yet. Re-exports are followed one hop.

### JSON schema (`--format json`, `version: 1`)

```jsonc
{
  "version": 1,
  "root": "…",
  "generated_at": 1730000000,
  "nodes": [
    { "id": "file:src/main.rs", "kind": "file", "label": "src/main.rs",
      "path": "src/main.rs", "language": "rust", "exported": true }
  ],
  "edges": [
    { "source": "file:src/main.rs", "target": "file:src/cli.rs",
      "kind": "imports", "confidence": 1.0 }
  ]
}
```

Node ids are stable: `file:<relpath>`, `sym:<relpath>#<name>@<line>`, `ext:<specifier>`.

---

## `dump` Output Format

The generated Markdown file follows this structure:

````markdown
# Directory Tree

```txt
├── Cargo.toml
└── src
     ├── main.rs
     └── cli.rs
```

---

# Source Files

## File: `src/main.rs`

```rs
// Your source code goes here...
```
