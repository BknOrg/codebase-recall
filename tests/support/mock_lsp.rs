//! A real subprocess mock: the same executable acts as driver and LSP server.
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::process::Command;

fn main() {
    if std::path::Path::new("mock-mode").exists() {
        server();
        return;
    }
    for mode in [
        "success",
        "init_error",
        "timeout",
        "indexing",
        "definition_error",
    ] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("mock-mode"), mode).unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("main.rs"),
            "pub fn alpha() {} pub fn bravo() {}\nfn main() { bravo(); }\n",
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
            .args([
                "sync",
                "--precise",
                "--precise-timeout",
                "1",
                "--no-report",
                "--project",
            ])
            .arg(temp.path())
            .env("CODE_RCL_LSP_RUST", std::env::current_exe().unwrap())
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            mode == "success",
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lease = std::fs::File::options()
            .read(true)
            .write(true)
            .open(temp.path().join("mock-lease"))
            .unwrap();
        assert!(lease.try_lock().is_ok(), "mock LSP leaked after {mode}");
        drop(lease);
        if mode != "success" {
            let fallback = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
                .args([
                    "sync",
                    "--json",
                    "--precise",
                    "--allow-heuristic-fallback",
                    "--precise-timeout",
                    "1",
                    "--no-report",
                    "--project",
                ])
                .arg(temp.path())
                .env("CODE_RCL_LSP_RUST", std::env::current_exe().unwrap())
                .output()
                .unwrap();
            assert!(
                fallback.status.success(),
                "{}",
                String::from_utf8_lossy(&fallback.stderr)
            );
            let result: Value = serde_json::from_slice(&fallback.stdout).unwrap();
            assert_eq!(result["status"], "partial");
            assert!(
                !result["metadata"]["warnings"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        if mode == "success" {
            let output = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
                .args([
                    "graph",
                    "--no-sync",
                    "--format",
                    "json",
                    "-o",
                    "-",
                    "--project",
                ])
                .arg(temp.path())
                .output()
                .unwrap();
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                value["data"]["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(
                        |e| e["source"].as_str().is_some_and(|s| s.contains("#main@"))
                            && e["target"].as_str().is_some_and(|s| s.contains("#bravo@"))
                            && e["confidence"] == 1.0
                    ),
                "same-line definition resolved incorrectly: {value}"
            );
        }
    }
    println!(
        "mock LSP: success, same-line definitions, initialization failure, request timeout, indexing timeout, definition error, and process cleanup passed"
    );
}

fn server() {
    let lease = std::fs::File::create("mock-lease").unwrap();
    lease.lock().unwrap();
    let mode = std::fs::read_to_string("mock-mode").unwrap();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    loop {
        let mut length = 0;
        loop {
            let mut header = String::new();
            if input.read_line(&mut header).unwrap() == 0 {
                return;
            }
            if header.trim().is_empty() {
                break;
            }
            if let Some(n) = header.strip_prefix("Content-Length:") {
                length = n.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        input.read_exact(&mut body).unwrap();
        let message: Value = serde_json::from_slice(&body).unwrap();
        match message["method"].as_str().unwrap_or("") {
            "initialize" if mode == "timeout" => {}
            "initialize" if mode == "init_error" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32603,"message":"mock initialization failed"}}),
            ),
            "initialize" => {
                send(json!({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{}}}))
            }
            "initialized" if mode == "indexing" => send(
                json!({"jsonrpc":"2.0","method":"$/progress","params":{"token":"index","value":{"kind":"begin","title":"indexing"}}}),
            ),
            "initialized" => send(
                json!({"jsonrpc":"2.0","method":"experimental/serverStatus","params":{"quiescent":true}}),
            ),
            "textDocument/definition" if mode == "definition_error" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32603,"message":"mock definition failed"}}),
            ),
            "textDocument/definition" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"result":{"uri":message["params"]["textDocument"]["uri"],"range":{"start":{"line":0,"character":24},"end":{"line":0,"character":29}}}}),
            ),
            "shutdown" => send(json!({"jsonrpc":"2.0","id":message["id"],"result":null})),
            "exit" => return,
            _ => {}
        }
    }
}
fn send(value: Value) {
    let body = value.to_string();
    let mut output = std::io::stdout().lock();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}
