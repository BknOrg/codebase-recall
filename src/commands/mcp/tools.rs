use serde_json::{Value, json};

pub fn list_tools() -> Value {
    json!([
        {
            "name": "code_rcl_digest",
            "description": "Generate an architecture outline, module summary, and public API index of the codebase (strips function bodies to save 80-90% prompt tokens). Detects Core Architecture Hubs with highest connectivity. Fast and lightweight.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Target project root directory. Defaults to the MCP server's configured project."
                    },
                    "path": {
                        "type": "string",
                        "description": "Sub-path within the project to outline (e.g. 'src/analysis'). Defaults to entire project."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "Include private/internal functions and types. Default is false (public/exported API only)."
                    },
                    "doc_lines": {
                        "type": "integer",
                        "description": "Retain up to N lines of doc comments per symbol (default 0). Fenced code diagrams are automatically stripped from text preview to conserve tokens."
                    },
                    "with_docs": {
                        "type": "boolean",
                        "description": "Convenience flag to retain doc comments (defaults to 3 lines)."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON data instead of markdown outline."
                    }
                }
            }
        },
        {
            "name": "code_rcl_impact",
            "description": "Analyze modification blast radius, upstream callers (who calls this), and downstream callees (what this calls) across the codebase before refactoring or editing symbols. Displays calling line snippets and edge confidence scores. Use diff=true to auto-detect targets from uncommitted git changes instead of typing a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name (e.g. 'build_graph' or 'MyStruct') or project-relative file path. Required unless `diff` is true."
                    },
                    "diff": {
                        "type": "boolean",
                        "description": "If true, detect symbols touched by uncommitted/staged git changes (vs HEAD) and use them as targets instead of `symbol`. Cannot be combined with `symbol`."
                    },
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Traversal hop depth for upstream and downstream branches (default 2, range 1-8)."
                    },
                    "direction": {
                        "type": "string",
                        "description": "Traversal direction: 'both' (default, 2 hops up & 2 hops down), 'reverse' (callers only), or 'forward' (callees only)."
                    },
                    "kinds": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Edge kinds to traverse. Defaults to ['calls', 'imports', 'references']; 'references' covers type positions (parameter, field, return type), so a struct used only as a parameter type still shows its users."
                    },
                    "precise": {
                        "type": "boolean",
                        "description": "If true, queries the installed compiler/LSP (gopls, rust-analyzer, pyright, etc.) to resolve exact ground-truth references before traversal. Default is false."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON report instead of formatted ASCII tree."
                    }
                }
            }
        },
        {
            "name": "code_rcl_path",
            "description": "Find the shortest chain of calls/imports connecting one symbol to another (how does A reach B?). Each hop shows the edge kind, confidence, and the source line. Returns found=false (not an error) when no chain exists; try direction='any' to follow edges both ways.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": {
                        "type": "string",
                        "description": "Symbol name or project-relative file path to start from."
                    },
                    "to": {
                        "type": "string",
                        "description": "Symbol name or project-relative file path to reach."
                    },
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "direction": {
                        "type": "string",
                        "description": "'forward' (default: FROM calls ... TO), 'reverse' (TO calls ... FROM), or 'any'."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum path length in hops (default 8, range 1-20)."
                    },
                    "kinds": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Edge kinds to follow. Defaults to ['calls', 'imports', 'references']."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON instead of formatted ASCII."
                    }
                },
                "required": ["from", "to"]
            }
        },
        {
            "name": "code_rcl_explain",
            "description": "Summarize one symbol in a single call: signature, doc comment, visibility, members (methods/fields), and its direct callers and callees with risk flags. Use this to understand a symbol before reading its file or editing it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name (e.g. 'build_graph' or 'MyStruct') or project-relative file path."
                    },
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON instead of formatted ASCII."
                    }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "code_rcl_report",
            "description": "One-page architecture overview of the project: core hubs, subsystems (groups of files that depend on each other, detected from imports and calls) with their key files and cohesion, the bridge files between subsystems, and suggested follow-up commands. Read this first when orienting in an unfamiliar codebase. The same report is kept fresh at .code-rcl/REPORT.md by sync.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON instead of Markdown."
                    }
                }
            }
        },
        {
            "name": "code_rcl_dump",
            "description": "Extract a relation-aware codebase context bundle. Given a focal symbol or file, extracts its connected graph neighborhood up to N hops into a token-efficient context markdown. Ideal for feeding focused context to LLMs without entire repo dumping.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Focal symbol name or relative file path to center the bundle around."
                    },
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Neighborhood hop depth (default 2, range 1-5)."
                    },
                    "max_size_kb": {
                        "type": "integer",
                        "description": "Skip files larger than this size in KB (default 50)."
                    }
                },
                "required": ["target"]
            }
        },
        {
            "name": "code_rcl_search",
            "description": "Fast symbol, function, struct, interface, and string-literal lookup from SQLite cache with fuzzy 'did you mean' suggestions and hybrid full-text grep fallback when exact symbols are not found.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Symbol name, substring, or config key/string literal to search for (e.g. 'CacheDb', 'get_bool', 'resolve')."
                    },
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Filter by entity kind: 'function', 'method', 'struct', 'interface', 'trait', 'enum', 'type', 'class', etc."
                    },
                    "exported_only": {
                        "type": "boolean",
                        "description": "If true, only return public / exported symbols. Default false."
                    },
                    "strings": {
                        "type": "boolean",
                        "description": "Also search string literals appearing as arguments in calls and macros (e.g. config keys or error messages)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default 25, max 100)."
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "code_rcl_sync",
            "description": "Synchronize modified files into the AST cache, with optional compiler-grade LSP resolution. Use precise=true ONLY when ground-truth verification is required for complex polymorphic dispatches (Go interfaces, Rust traits) before critical refactoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "precise": {
                        "type": "boolean",
                        "description": "If true, launches compiler language servers (gopls, rust-analyzer, pyright, etc.) to resolve exact ground-truth references. Leave false for instant incremental AST sync."
                    },
                    "languages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter specific languages for precise resolution (e.g. ['go', 'rust'])."
                    },
                    "full": {
                        "type": "boolean",
                        "description": "Re-resolve every reference even if already up to date in cache (default false)."
                    }
                }
            }
        },
        {
            "name": "code_rcl_graph",
            "description": "Query the code relation graph (nodes and edges) in structured JSON format for deep dependency inspection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Target project root directory."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["file", "symbol", "both"],
                        "description": "Graph node granularity (default: 'both')."
                    },
                    "focus": {
                        "type": "string",
                        "description": "Center the graph on a focal symbol or file."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "BFS hop depth around focus (default 2)."
                    },
                    "precise": {
                        "type": "boolean",
                        "description": "If true, runs compiler-grade LSP resolution before building the graph. Default is false."
                    }
                }
            }
        }
    ])
}
