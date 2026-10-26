//! Model Context Protocol (MCP) Server for `code-rcl`.
//!
//! Exposes codebase analysis tools (`digest`, `impact`, `dump`, `graph`, `search`, `sync`)
//! over standard JSON-RPC 2.0 via `stdio` transport.

mod budget;
pub mod handlers;
pub mod inputs;
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
    use std::collections::HashMap;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let pending = Arc::new(Mutex::new(HashMap::<String, Arc<AtomicBool>>::new()));
    let (sender, receiver) = mpsc::channel::<JsonRpcRequest>();
    let project = args.project.unwrap_or_default();
    let worker_output = output.clone();
    let worker_pending = pending.clone();
    let worker = std::thread::spawn(move || -> Result<()> {
        for request in receiver {
            let key = request.id.as_ref().unwrap().to_string();
            let cancel = worker_pending.lock().unwrap().get(&key).cloned().unwrap();
            crate::operation::begin(cancel.clone());
            let progress_token = request
                .params
                .as_ref()
                .and_then(|p| p.pointer("/_meta/progressToken"))
                .filter(|v| v.is_string() || v.is_number())
                .cloned();
            let progress_output = worker_output.clone();
            crate::progress::sink(progress_token.map(|token| std::sync::Arc::new(move |value: Value| {
                let message = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":value["elapsed_ms"],"message":value.to_string()}});
                let mut output = progress_output.lock().unwrap();
                let _ = writeln!(output,"{message}"); let _ = output.flush();
            }) as crate::progress::Sink));
            let response = if cancel.load(Ordering::Relaxed) {
                None
            } else {
                handle_request(&project, request)
            };
            if !cancel.load(Ordering::Relaxed)
                && let Some(response) = response
            {
                send_response(&mut worker_output.lock().unwrap(), &response)?;
            }
            crate::progress::sink(None);
            worker_pending.lock().unwrap().remove(&key);
        }
        Ok(())
    });
    let mut initialized = false;
    let mut ready = false;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw: Value = match serde_json::from_str(&line) {
            Ok(raw) => raw,
            Err(error) => {
                send_response(
                    &mut output.lock().unwrap(),
                    &rpc_error(None, -32700, error.to_string()),
                )?;
                continue;
            }
        };
        let id = raw
            .get("id")
            .filter(|id| id.is_string() || id.is_number())
            .cloned();
        if raw.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || !raw["method"].is_string()
            || (raw.get("id").is_some() && id.is_none())
        {
            send_response(
                &mut output.lock().unwrap(),
                &rpc_error(id, -32600, "Invalid JSON-RPC request".into()),
            )?;
            continue;
        }
        let request: JsonRpcRequest = serde_json::from_value(raw)?;
        if request.id.is_none() {
            match request.method.as_str() {
                "notifications/initialized" if initialized => ready = true,
                "notifications/cancelled" => {
                    if let Some(id) = request.params.as_ref().and_then(|p| p.get("requestId"))
                        && let Some(cancel) = pending.lock().unwrap().get(&id.to_string())
                    {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
            continue;
        }
        let response = if request.method == "initialize" {
            if initialized {
                Some(rpc_error(request.id, -32600, "Already initialized".into()))
            } else if request
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .is_none()
            {
                Some(rpc_error(
                    request.id,
                    -32602,
                    "protocolVersion is required".into(),
                ))
            } else {
                initialized = true;
                handle_request(Path::new(""), request)
            }
        } else if request.method != "ping" && !ready {
            Some(rpc_error(
                request.id,
                -32600,
                "Initialize and send notifications/initialized first".into(),
            ))
        } else if request.method == "tools/call" {
            let params = request.params.as_ref().unwrap_or(&Value::Null);
            let name = params["name"].as_str().unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Err(error) = inputs::validate(name, &args) {
                Some(rpc_error(request.id, -32602, error.to_string()))
            } else {
                let key = request.id.as_ref().unwrap().to_string();
                let mut pending = pending.lock().unwrap();
                if let std::collections::hash_map::Entry::Vacant(entry) = pending.entry(key) {
                    entry.insert(Arc::new(AtomicBool::new(false)));
                    sender.send(request)?;
                    None
                } else {
                    Some(rpc_error(
                        request.id,
                        -32600,
                        "Duplicate in-flight request ID".into(),
                    ))
                }
            }
        } else {
            handle_request(Path::new(""), request)
        };
        if let Some(response) = response {
            send_response(&mut output.lock().unwrap(), &response)?;
        }
    }
    drop(sender);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("MCP worker panicked"))??;
    Ok(())
}

fn rpc_error(id: Option<Value>, code: i64, message: String) -> JsonRpcResponse<'static> {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: Some(id.unwrap_or(Value::Null)),
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    }
}

pub fn send_response(stdout: &mut std::io::Stdout, resp: &JsonRpcResponse) -> Result<()> {
    let serialized = serde_json::to_vec(resp)?;
    stdout.write_all(&serialized)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

pub fn handle_request(
    default_project: &Path,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse<'static>> {
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
                },
                "instructions": "Start with report for architecture or search/explain for a symbol. Inspect metadata warnings, freshness, resolution_mode, and truncated before drawing conclusions. Empty graph results do not prove absence of dependencies. Read relevant source before editing."
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
