//! Model Context Protocol (MCP) Server for `code-rcl`.
//!
//! Exposes codebase analysis tools (`digest`, `impact`, `dump`, `graph`, `search`, `sync`)
//! over standard JSON-RPC 2.0 via `stdio` transport.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cache::CacheDb;
use crate::cli::{DigestArgs, GraphQuery, ImpactArgs, McpArgs, PreciseArgs};
use crate::commands::{digest, dump, graph, impact, sync};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "code-rcl";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse<'a> {
    pub jsonrpc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub fn run(args: McpArgs) -> Result<()> {
    eprintln!(
        "code-rcl MCP server starting (version {SERVER_VERSION}) for project: {}",
        args.project.display()
    );

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("MCP stdin read error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                        data: None,
                    }),
                };
                send_response(&mut stdout, &err_resp);
                continue;
            }
        };

        let is_notification = req.id.is_none();
        let resp = handle_request(&args.project, req);

        if !is_notification {
            if let Some(r) = resp {
                send_response(&mut stdout, &r);
            }
        }
    }

    eprintln!("code-rcl MCP server exited.");
    Ok(())
}

fn send_response(stdout: &mut std::io::Stdout, resp: &JsonRpcResponse) {
    if let Ok(serialized) = serde_json::to_string(resp) {
        let _ = stdout.write_all(serialized.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}

fn handle_request(default_project: &Path, req: JsonRpcRequest) -> Option<JsonRpcResponse<'static>> {
    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => {
            let result = json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                }
            });
            Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            })
        }
        "notifications/initialized" | "initialized" => {
            // Notification: no response needed
            None
        }
        "ping" => Some(JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({})),
            error: None,
        }),
        "tools/list" => {
            let tools = list_tools();
            Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({ "tools": tools })),
                error: None,
            })
        }
        "tools/call" => {
            let params = req.params.unwrap_or(Value::Null);
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            let call_result = call_tool(default_project, tool_name, &arguments);
            Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(call_result),
                error: None,
            })
        }
        _ => Some(JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
                data: None,
            }),
        }),
    }
}

fn list_tools() -> Value {
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
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON data instead of markdown outline."
                    }
                }
            }
        },
        {
            "name": "code_rcl_impact",
            "description": "Analyze modification blast radius and reverse caller hierarchy (who calls/imports this symbol or file) across the codebase before refactoring or editing symbols. By default uses fast heuristics and cached compiler data; set precise=true only when verifying complex interface or trait dispatch with real compilers.",
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
                    "depth": {
                        "type": "integer",
                        "description": "Reverse traversal hop depth (default 3, range 1-8)."
                    },
                    "kinds": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Edge kinds to traverse: ['calls', 'imports', 'references']."
                    },
                    "precise": {
                        "type": "boolean",
                        "description": "If true, queries the installed compiler/LSP (gopls, rust-analyzer, pyright, etc.) to resolve exact ground-truth references before traversal. Default is false."
                    },
                    "json": {
                        "type": "boolean",
                        "description": "If true, return structured JSON report instead of formatted ASCII tree."
                    }
                },
                "required": ["symbol"]
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
            "description": "Fast symbol and declaration lookup across the codebase from the SQLite cache (instant, 0-5ms). Finds functions, structs, interfaces, methods, traits, types, or enums by name or substring without manual file grepping.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Symbol name or substring to search for (e.g. 'CacheDb', 'build_graph', 'resolve')."
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

fn call_tool(default_project: &Path, name: &str, args: &Value) -> Value {
    let result = match name {
        "code_rcl_digest" => execute_digest(default_project, args),
        "code_rcl_impact" => execute_impact(default_project, args),
        "code_rcl_dump" => execute_dump(default_project, args),
        "code_rcl_search" => execute_search(default_project, args),
        "code_rcl_sync" => execute_sync(default_project, args),
        "code_rcl_graph" => execute_graph(default_project, args),
        _ => Err(anyhow::anyhow!("Unknown tool: {name}")),
    };

    match result {
        Ok(text) => json!({
            "content": [
                {
                    "type": "text",
                    "text": text
                }
            ],
            "isError": false
        }),
        Err(err) => json!({
            "content": [
                {
                    "type": "text",
                    "text": format!("Error: {err:#}")
                }
            ],
            "isError": true
        }),
    }
}

fn resolve_project(default_project: &Path, args: &Value) -> PathBuf {
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    default_project.to_path_buf()
}

fn execute_digest(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let sub_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let digest_args = DigestArgs {
        path: sub_path,
        project: Some(project),
        output: None,
        all,
        json: as_json,
        no_sync: false,
    };

    let report = digest::generate_digest(&digest_args)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&report)?)
    } else {
        Ok(digest::format_markdown_digest(&report))
    }
}

fn execute_impact(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'symbol'");
    };

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 8))
        .unwrap_or(3);

    let kinds: Vec<String> = args
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["calls".to_string(), "imports".to_string()]);

    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);
    let precise = args.get("precise").and_then(|v| v.as_bool()).unwrap_or(false);

    let impact_args = ImpactArgs {
        symbol: symbol.to_string(),
        project,
        depth,
        kinds,
        json: as_json,
        no_sync: false,
        precise: PreciseArgs {
            precise,
            precise_full: false,
            precise_timeout: 15,
        },
    };

    let reports = impact::generate_reports(&impact_args)?;
    if as_json {
        impact::render_json(&reports)
    } else {
        Ok(impact::render_ascii(&reports))
    }
}

fn execute_dump(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(target) = args.get("target").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'target'");
    };

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 5))
        .unwrap_or(2);

    let max_size_kb = args
        .get("max_size_kb")
        .and_then(|v| v.as_u64())
        .unwrap_or(50);

    let (content, _) = dump::generate_relation_bundle(&project, target, depth, max_size_kb, false)?;
    Ok(content)
}

fn execute_search(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'query'");
    };
    let kind = args.get("kind").and_then(|v| v.as_str());
    let exported_only = args.get("exported_only").and_then(|v| v.as_bool()).unwrap_or(false);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25).clamp(1, 100) as usize;

    let db = CacheDb::open(&project)?;
    let results = db.search_symbols(query, kind, exported_only, limit)?;

    if results.is_empty() {
        return Ok(format!("No symbols found matching '{query}'."));
    }

    let mut out = format!("Found {} symbol(s) matching `{query}`:\n\n", results.len());
    out.push_str("| Symbol | Kind | Location | Visibility | Signature |\n");
    out.push_str("| :--- | :--- | :--- | :--- | :--- |\n");

    for (sym, path) in results {
        let vis = if sym.is_exported { "public" } else { "private" };
        let line_range = match (sym.start_line, sym.end_line) {
            (Some(s), Some(e)) if s == e => format!("L{s}"),
            (Some(s), Some(e)) => format!("L{s}-{e}"),
            _ => "-".to_string(),
        };
        let sig = sym.signature.unwrap_or_else(|| sym.name.clone());
        out.push_str(&format!(
            "| **`{}`** | `{}` | `{}:{}` | {} | `{}` |\n",
            sym.name, sym.kind, path, line_range, vis, sig
        ));
    }

    Ok(out)
}

fn execute_sync(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let mut db = CacheDb::open(&project)?;
    let precise = args.get("precise").and_then(|v| v.as_bool()).unwrap_or(false);
    let full = args.get("full").and_then(|v| v.as_bool()).unwrap_or(false);
    let languages: Vec<String> = args
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let sync_args = crate::cli::SyncArgs {
        project: project.clone(),
        max_file_kb: 512,
        language: languages,
        precise: PreciseArgs {
            precise,
            precise_full: full,
            precise_timeout: 15,
        },
    };

    let stats = sync::sync_cache(&mut db, &sync_args)?;
    let mut out = format!(
        "Sync completed: {} scanned (+{} added, ~{} changed, ={} unchanged, -{} removed) ({} symbols, {} imports)",
        stats.scanned, stats.added, stats.changed, stats.unchanged, stats.removed, stats.symbols, stats.imports
    );

    if precise {
        let precise_stats = sync::run_precise(&mut db, &sync_args)?;
        sync::report_precise(&precise_stats);

        out.push_str("\n\nPrecise Resolution (L0 Ground Truth):\n");
        for outcome in &precise_stats.languages {
            out.push_str(&format!(
                "- **{}** (via `{}`): resolved {}/{} refs in {} file(s) ({} external, {} unresolved)\n",
                outcome.language,
                outcome.server,
                outcome.hits,
                outcome.queried,
                outcome.files,
                outcome.external,
                outcome.unresolved,
            ));
        }
        if precise_stats.languages.is_empty() {
            out.push_str("- All references were already up to date in cache.\n");
        }
        if !precise_stats.warnings.is_empty() {
            out.push_str("\nWarnings:\n");
            for w in &precise_stats.warnings {
                out.push_str(&format!("- {w}\n"));
            }
        }
    }

    Ok(out)
}

fn execute_graph(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .to_string();

    let focus = args
        .get("focus")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32)
        .unwrap_or(2);

    let precise = args.get("precise").and_then(|v| v.as_bool()).unwrap_or(false);

    let query = GraphQuery {
        project,
        scope,
        kinds: vec![
            "imports".to_string(),
            "calls".to_string(),
            "contains".to_string(),
        ],
        path: None,
        focus,
        depth,
        min_confidence: 0.0,
        include_external: false,
        max_nodes: 2000,
        no_sync: false,
        precise: PreciseArgs {
            precise,
            precise_full: false,
            precise_timeout: 15,
        },
    };

    let code_graph = graph::build_graph(&query)?;
    Ok(serde_json::to_string_pretty(&code_graph)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: Some(json!({})),
        };
        let resp = handle_request(Path::new("."), req).expect("response");
        assert_eq!(resp.id, Some(json!(1)));
        let result = resp.result.expect("result");
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "code-rcl");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn test_ping() {
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-42")),
            method: "ping".to_string(),
            params: None,
        };
        let resp = handle_request(Path::new("."), req).expect("response");
        assert_eq!(resp.id, Some(json!("req-42")));
        assert_eq!(resp.result, Some(json!({})));
    }

    #[test]
    fn test_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(2)),
            method: "tools/list".to_string(),
            params: None,
        };
        let resp = handle_request(Path::new("."), req).expect("response");
        let result = resp.result.expect("result");
        let tools = result["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 6);

        let names: Vec<_> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"code_rcl_digest"));
        assert!(names.contains(&"code_rcl_impact"));
        assert!(names.contains(&"code_rcl_dump"));
        assert!(names.contains(&"code_rcl_search"));
        assert!(names.contains(&"code_rcl_sync"));
        assert!(names.contains(&"code_rcl_graph"));
    }

    #[test]
    fn test_unknown_method() {
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(99)),
            method: "non_existent_method".to_string(),
            params: None,
        };
        let resp = handle_request(Path::new("."), req).expect("response");
        assert_eq!(resp.id, Some(json!(99)));
        let err = resp.error.expect("error object");
        assert_eq!(err.code, -32601);
    }
}
