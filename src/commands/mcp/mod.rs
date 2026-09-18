//! Model Context Protocol (MCP) Server for `code-rcl`.
//!
//! Exposes codebase analysis tools (`digest`, `impact`, `dump`, `graph`, `search`, `sync`)
//! over standard JSON-RPC 2.0 via `stdio` transport.

pub mod handlers;
pub mod protocol;
pub mod tools;

pub use handlers::*;
pub use protocol::*;
pub use tools::*;

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::McpArgs;

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

pub fn send_response(stdout: &mut std::io::Stdout, resp: &JsonRpcResponse) {
    if let Ok(serialized) = serde_json::to_string(resp) {
        let _ = stdout.write_all(serialized.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}

pub fn handle_request(default_project: &Path, req: JsonRpcRequest) -> Option<JsonRpcResponse<'static>> {
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
        assert_eq!(tools.len(), 9);

        let names: Vec<_> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"code_rcl_digest"));
        assert!(names.contains(&"code_rcl_impact"));
        assert!(names.contains(&"code_rcl_path"));
        assert!(names.contains(&"code_rcl_explain"));
        assert!(names.contains(&"code_rcl_report"));
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
