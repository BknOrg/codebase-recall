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
                .env("CODE_RCL_LSP_RUST", std::env::current_exe().unwrap())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
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
    c_family_contract();
    selective_contract();
    selective_language_contract();
    vue_bridge_contract();
    selective_error_contract();
    selective_batch_contract();
    selective_cancel_contract();
    println!(
        "mock LSP: success, same-line definitions, initialization failure, request timeout, indexing timeout, definition error, and process cleanup passed"
    );
}

fn c_family_contract() {
    for mode in [
        "c_success",
        "c_multi",
        "c_multi_same_node",
        "c_external",
        "c_diagnostics",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("mock-mode"), mode).unwrap();
        std::fs::write(
            root.join("main.cpp"),
            "#include \"api.h\"\nint main(){return chosen();}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("api.h"),
            "int chosen(){return 1;} int other(){return 2;}\n",
        )
        .unwrap();
        std::fs::write(root.join("compile_flags.txt"), "-std=c++23\n").unwrap();
        let exe = std::env::current_exe().unwrap();
        let invoke = |extra: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_code-rcl"))
                .args([
                    "graph",
                    "--format",
                    "json",
                    "-o",
                    "-",
                    "--min-confidence",
                    "0",
                    "--project",
                ])
                .arg(root)
                .args(extra)
                .env("CODE_RCL_LSP_CLANGD", &exe)
                .output()
                .unwrap()
        };
        let out = invoke(&["--precise"]);
        if mode == "c_diagnostics" {
            assert!(!out.status.success());
            assert!(String::from_utf8_lossy(&out.stderr).contains("clangd reported errors"));
            let fallback = invoke(&["--precise", "--allow-heuristic-fallback"]);
            assert!(fallback.status.success());
            let v: Value = serde_json::from_slice(&fallback.stdout).unwrap();
            assert_eq!(v["status"], "partial");
            continue;
        }
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let calls: Vec<_> = v["data"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "calls")
            .collect();
        if mode == "c_success" {
            assert!(
                calls.iter().any(|e| e["confidence"] == 1.0
                    && e["target"].as_str().unwrap().contains("#chosen@")),
                "{v}"
            );
        } else {
            assert!(
                calls.is_empty(),
                "{mode}: local guesses must not replace LSP results: {v}"
            );
        }
        let opens = std::fs::read_to_string(root.join("mock-opened.log")).unwrap();
        assert!(
            opens.lines().next().unwrap().contains("main.cpp"),
            "translation unit must open before header: {opens}"
        );
        let args = std::fs::read_to_string(root.join("mock-args.log")).unwrap();
        assert!(args.contains("--compile-commands-dir="));
        assert!(!args.contains("--query-driver="));
        assert!(invoke(&["--no-sync"]).status.success());
        std::fs::write(root.join("compile_flags.txt"), "-std=c++23\n-DCHANGED=1\n").unwrap();
        assert!(!invoke(&["--no-sync"]).status.success());
        assert!(invoke(&["--precise"]).status.success());
    }
}

fn server() {
    std::fs::write(
        "mock-args.log",
        std::env::args().collect::<Vec<_>>().join("\n"),
    )
    .unwrap();
    let mut starts = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("mock-starts.log")
        .unwrap();
    writeln!(starts, "start").unwrap();
    let vue = std::env::args().any(|a| a.starts_with("--tsdk="));
    let mode = std::fs::read_to_string("mock-mode").unwrap();
    let lease = std::fs::File::create(if mode == "vue_bridge" && vue {
        "mock-lease-vue"
    } else {
        "mock-lease"
    })
    .unwrap();
    lease.lock().unwrap();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut waiting: Option<Value> = None;
    let mut definitions = 0;
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
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("mock-requests.log")
            .unwrap();
        writeln!(log, "{message}").unwrap();
        match message["method"].as_str().unwrap_or("") {
            "initialize" if mode == "malformed" => {
                let mut out = std::io::stdout().lock();
                write!(out, "Content-Length: 3\r\n\r\nxxx").unwrap();
                out.flush().unwrap();
            }
            "initialize" if mode == "timeout" => {}
            "initialize" if mode == "init_error" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32603,"message":"mock initialization failed"}}),
            ),
            "initialize" if mode == "unsupported" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{"definitionProvider":false}}}),
            ),
            "initialize" if mode == "crash" => return,
            "initialize" if mode == "vue_bridge" && !vue => send(
                json!({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{"executeCommandProvider":{"commands":["typescript.tsserverRequest"]}}}}),
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
            "textDocument/didOpen" => {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("mock-opened.log")
                    .unwrap();
                writeln!(file, "{}", message["params"]["textDocument"]["uri"]).unwrap();
                if mode == "c_diagnostics" {
                    send(
                        json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":message["params"]["textDocument"]["uri"],"diagnostics":[{"severity":1,"message":"missing configured header"}]}}),
                    );
                }
            }
            "workspace/executeCommand" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"result":{"body":{"mockBridge":true}}}),
            ),
            "textDocument/definition" if mode == "vue_bridge" => {
                waiting = Some(message.clone());
                send(
                    json!({"jsonrpc":"2.0","method":"tsserver/request","params":[123,"projectInfo",{"file":"App.vue"}]}),
                );
            }
            "tsserver/response" => {
                assert_eq!(message["params"][1]["mockBridge"], true);
                let request = waiting.take().unwrap();
                send(
                    json!({"jsonrpc":"2.0","id":request["id"],"result":{"uri":"vue-virtual:///App.vue.ts","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}}),
                );
            }
            "textDocument/definition" if mode == "empty" => {
                send(json!({"jsonrpc":"2.0","id":message["id"],"result":null}))
            }
            "textDocument/definition" if mode == "partial_batch" => {
                definitions += 1;
                if definitions == 1 {
                    send(
                        json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32603,"message":"one failed request"}}),
                    );
                } else {
                    send(json!({"jsonrpc":"2.0","id":message["id"],"result":null}));
                }
            }
            "textDocument/definition" if mode == "definition_error" => send(
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32603,"message":"mock definition failed"}}),
            ),
            "textDocument/definition" if mode.starts_with("c_") => {
                let uri = message["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap()
                    .replace("main.cpp", "api.h");
                let location = |uri: &str, at: usize| json!({"uri":uri,"range":{"start":{"line":0,"character":at},"end":{"line":0,"character":at+1}}});
                let result = match mode.as_str() {
                    "c_multi" => json!([location(&uri, 4), location(&uri, 27)]),
                    "c_multi_same_node" => json!([location(&uri, 4), location(&uri, 5)]),
                    "c_external" => location("file:///external/dependency.hpp", 4),
                    _ => location(&uri, 4),
                };
                send(json!({"jsonrpc":"2.0","id":message["id"],"result":result}));
            }
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
fn selective_contract() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("mock-mode"), "success").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.rs"),
        "pub fn alpha() {} pub fn bravo() {}\nfn main(){ bravo(); missing(); }\n",
    )
    .unwrap();
    std::fs::create_dir(root.join(".code-rcl")).unwrap();
    std::fs::write(
        root.join(".code-rcl/config.toml"),
        "[precise]\nselective_budget_secs=1\n",
    )
    .unwrap();
    let exe = std::env::current_exe().unwrap();
    let invoke = |flags: &[&str], server: &std::path::Path| {
        let out = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
            .args(["sync", "--json", "--no-report", "--project"])
            .arg(root)
            .args(flags)
            .env("CODE_RCL_LSP_RUST", server)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let queries = || {
        std::fs::read_to_string(root.join("mock-requests.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("textDocument/definition"))
            .count()
    };
    // Plain sync is entirely offline, including with an invalid override.
    invoke(&[], std::path::Path::new("missing-server.exe"));
    assert_eq!(queries(), 0);
    let cold = invoke(&["--precise-selective"], &exe);
    assert_eq!(cold["status"], "ok", "{cold}");
    let c = &cold["metadata"]["precision"]["languages"][0];
    assert!(c["skipped"].as_u64().unwrap() > 0, "{cold}");
    assert!(c["queried"].as_u64().unwrap() > 0, "{cold}");
    let mut selective_count = queries();
    assert_eq!(cold["metadata"]["resolution_mode"], "mixed");
    let warm = invoke(&["--precise-selective"], &exe);
    assert_eq!(queries(), selective_count);
    assert!(
        warm["metadata"]["precision"]["languages"][0]["cache_hits"]
            .as_u64()
            .unwrap()
            > 0
    );
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.2.0'\n",
    )
    .unwrap();
    let stale = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args([
            "graph",
            "--no-sync",
            "--format",
            "json",
            "-o",
            "-",
            "--project",
        ])
        .arg(root)
        .env("CODE_RCL_LSP_RUST", &exe)
        .output()
        .unwrap();
    assert!(!stale.status.success());
    invoke(&["--precise-selective"], &exe);
    assert!(queries() > selective_count);
    selective_count = queries();
    let full = invoke(&["--precise"], &exe);
    assert!(queries() > selective_count);
    assert_eq!(full["metadata"]["resolution_mode"], "precise_cached");
    let full_count = queries();
    invoke(&["--precise-selective"], &exe);
    assert_eq!(queries(), full_count);
    invoke(&["--precise-selective", "--precise-full"], &exe);
    assert!(queries() > full_count);
    let failed = invoke(
        &["--precise-selective", "--precise-full"],
        std::path::Path::new("missing-server.exe"),
    );
    assert_eq!(failed["status"], "partial");
    assert!(
        failed["metadata"]["warnings"]
            .to_string()
            .contains("server_missing")
    );
    // Timeout consumes one operation-wide budget; no process survives it.
    std::fs::write(root.join("mock-mode"), "timeout").unwrap();
    let started = std::time::Instant::now();
    let timeout = invoke(&["--precise-selective", "--precise-full"], &exe);
    assert_eq!(timeout["status"], "partial");
    assert!(started.elapsed().as_secs_f64() < 5.0);
    let starts = std::fs::read_to_string(root.join("mock-starts.log"))
        .unwrap()
        .lines()
        .count();
    let cooldown = invoke(&["--precise-selective"], &exe);
    assert!(cooldown.to_string().contains("cooldown"));
    assert_eq!(
        std::fs::read_to_string(root.join("mock-starts.log"))
            .unwrap()
            .lines()
            .count(),
        starts
    );
    let lease = std::fs::File::options()
        .read(true)
        .write(true)
        .open(root.join("mock-lease"))
        .unwrap();
    assert!(lease.try_lock().is_ok());
    drop(lease);
    // No candidate means no executable discovery, even with a missing server.
    std::fs::write(
        root.join("main.rs"),
        "pub fn alpha() {} pub fn bravo() {}\nfn main(){ bravo(); }\n",
    )
    .unwrap();
    let empty = invoke(
        &["--precise-selective"],
        std::path::Path::new("missing-server.exe"),
    );
    assert_eq!(empty["status"], "ok", "{empty}");
    let conflict = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args(["sync", "--precise", "--precise-selective"])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    let missing_mode = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args(["sync", "--precise-full"])
        .output()
        .unwrap();
    assert!(!missing_mode.status.success());
    println!(
        "selective mock: cold/warm cache, selective/full transitions, forced refresh, missing executable, total deadline, cooldown, cleanup, empty selection and mode conflict passed"
    );
}
fn selective_language_contract() {
    for (language, name, source, env) in [
        ("rust", "main.rs", "fn main(){unknown();}", "RUST"),
        (
            "go",
            "main.go",
            "package main\nfunc main(){unknown()}\n",
            "GO",
        ),
        (
            "java",
            "Main.java",
            "class Main {void run(){unknown();}}",
            "JAVA",
        ),
        ("kotlin", "main.kt", "fun main(){unknown()}", "KOTLIN"),
        ("python", "main.py", "def run():\n    unknown()\n", "PYTHON"),
        (
            "javascript",
            "main.jsx",
            "export function run(){unknown();return <div/>}",
            "JAVASCRIPT",
        ),
        (
            "typescript",
            "main.tsx",
            "export function run(){unknown();return <div/>}",
            "TYPESCRIPT",
        ),
        (
            "svelte",
            "App.svelte",
            "<script>function run(){unknown()}</script><button on:click={run}>go</button>",
            "SVELTE",
        ),
        ("c", "main.c", "int main(void){unknown();}", "CLANGD"),
        ("cpp", "main.cpp", "int main(){unknown();}", "CLANGD"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join(name), source).unwrap();
        for (name, text) in [
            ("mock-mode", "empty"),
            ("Cargo.toml", "[package]\nname='fixture'\nversion='0.1.0'\n"),
            ("go.mod", "module fixture\ngo 1.22\n"),
            ("pom.xml", "<project/>"),
            ("build.gradle.kts", ""),
            (
                "compile_flags.txt",
                if language == "c" {
                    "-std=c17\n"
                } else {
                    "-std=c++23\n"
                },
            ),
        ] {
            std::fs::write(root.join(name), text).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
            .args([
                "sync",
                "--json",
                "--precise-selective",
                "--no-report",
                "--project",
            ])
            .arg(root)
            .env(
                format!("CODE_RCL_LSP_{env}"),
                std::env::current_exe().unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{language}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let v: Value = serde_json::from_slice(&output.stdout).unwrap();
        let c = v["metadata"]["precision"]["languages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["language"] == language)
            .unwrap();
        assert!(c["queried"].as_u64().unwrap() > 0, "{language}: {v}");
        assert_eq!(c["failed"], 0, "{language}: {v}");
        assert_eq!(c["verified"], 0);
        assert!(c["unresolved"].as_u64().unwrap() > 0);
        let log = std::fs::read_to_string(root.join("mock-requests.log")).unwrap();
        if name.ends_with("jsx") {
            assert!(log.contains("javascriptreact"));
        }
        if name.ends_with("tsx") {
            assert!(log.contains("typescriptreact"));
        }
    }
    // A failed backend cannot erase a successful backend's answer.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("mock-mode"), "empty").unwrap();
    std::fs::write(root.join("main.py"), "unknown()\n").unwrap();
    std::fs::write(root.join("main.js"), "unknown();\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args([
            "sync",
            "--json",
            "--precise-selective",
            "--no-report",
            "--project",
        ])
        .arg(root)
        .env("CODE_RCL_LSP_PYTHON", std::env::current_exe().unwrap())
        .env("CODE_RCL_LSP_JAVASCRIPT", root.join("missing.exe"))
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "partial");
    assert_eq!(v["metadata"]["resolution_mode"], "mixed");
    println!(
        "selective mock: every source family, React language IDs, empty answers and backend failure isolation passed"
    );
}
fn vue_bridge_contract() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(
        root.join("App.vue"),
        "<script setup lang=\"ts\">unknown()</script><template><button>go</button></template>",
    )
    .unwrap();
    std::fs::write(root.join("mock-mode"), "vue_bridge").unwrap();
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_code-rcl"))
            .args([
                "sync",
                "--json",
                "--precise-selective",
                "--no-report",
                "--project",
            ])
            .arg(root)
            .env("CODE_RCL_LSP_VUE", std::env::current_exe().unwrap())
            .env("CODE_RCL_LSP_TYPESCRIPT", std::env::current_exe().unwrap())
            .output()
            .unwrap()
    };
    let missing = invoke();
    assert!(missing.status.success());
    let v: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(v["status"], "partial");
    assert!(v.to_string().contains("dependency_missing"));
    assert!(!root.join("mock-starts.log").exists());
    let sdk = root.join("node_modules/typescript/lib");
    let plugin = root.join("node_modules/@vue/typescript-plugin");
    std::fs::create_dir_all(&sdk).unwrap();
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(sdk.join("typescript.js"), "").unwrap();
    std::fs::write(sdk.join("tsserver.js"), "").unwrap();
    std::fs::write(plugin.join("package.json"), "{}").unwrap();
    let output = invoke();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["status"], "ok", "{v}");
    let log = std::fs::read_to_string(root.join("mock-requests.log")).unwrap();
    assert!(log.contains("typescript.tsserverRequest"));
    assert!(log.contains("tsserver/response"));
    assert!(log.contains("@vue/typescript-plugin"));
    assert!(!log.contains("hybridMode"));
    assert_eq!(
        std::fs::read_to_string(root.join("mock-starts.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    println!(
        "Vue mock: missing SDK/plugin recovery, TypeScript companion bridge and virtual non-node answer passed"
    );
}
fn selective_error_contract() {
    for mode in [
        "init_error",
        "timeout",
        "indexing",
        "definition_error",
        "unsupported",
        "crash",
        "malformed",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("main.py"), "unknown()\n").unwrap();
        std::fs::write(root.join("mock-mode"), mode).unwrap();
        std::fs::create_dir(root.join(".code-rcl")).unwrap();
        std::fs::write(
            root.join(".code-rcl/config.toml"),
            "[precise]\nselective_budget_secs=1\n",
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
            .args([
                "sync",
                "--json",
                "--precise-selective",
                "--precise-timeout",
                "1",
                "--no-report",
                "--project",
            ])
            .arg(root)
            .env("CODE_RCL_LSP_PYTHON", std::env::current_exe().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["status"], "partial", "{mode}: {value}");
        assert_eq!(value["metadata"]["resolution_mode"], "heuristic");
        let lease = std::fs::File::options()
            .read(true)
            .write(true)
            .open(root.join("mock-lease"))
            .unwrap();
        assert!(lease.try_lock().is_ok(), "{mode} leaked server");
    }
    println!(
        "selective mock: initialization, timeout, indexing, request, unsupported capability, crash and protocol errors passed"
    );
}
fn selective_batch_contract() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("mock-mode"), "partial_batch").unwrap();
    let source = (0..80)
        .map(|i| format!("unknown_{i}()\n"))
        .collect::<String>();
    std::fs::write(root.join("main.py"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args([
            "sync",
            "--json",
            "--precise-selective",
            "--no-report",
            "--project",
        ])
        .arg(root)
        .env("CODE_RCL_LSP_PYTHON", std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    let c = &v["metadata"]["precision"]["languages"][0];
    assert_eq!(v["status"], "partial");
    assert_eq!(c["queried"], 80, "{v}");
    assert_eq!(c["unresolved"], 79);
    assert_eq!(c["failed"], 1);
    assert_eq!(c["pending"], 1);
    assert_eq!(c["server_starts"], 1);
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
        .arg(root)
        .env("CODE_RCL_LSP_PYTHON", std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "selective mock: 80-position batching retains 79 successful empty answers despite one request failure"
    );
}
fn selective_cancel_contract() {
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("main.py"), "unknown()\n").unwrap();
    std::fs::write(root.join("mock-mode"), "timeout").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_code-rcl"))
        .args(["mcp", "--project"])
        .arg(root)
        .env("CODE_RCL_LSP_PYTHON", std::env::current_exe().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"mock","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"code_rcl_sync","arguments":{"precise_selective":true}}}),
    ] {
        writeln!(input, "{msg}").unwrap();
    }
    input.flush().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.join("mock-lease").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        root.join("mock-lease").exists(),
        "selective LSP never started"
    );
    writeln!(input,"{}",json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2,"reason":"test cancellation"}})).unwrap();
    drop(input);
    let started = std::time::Instant::now();
    let output = child.wait_with_output().unwrap();
    assert!(started.elapsed().as_secs_f32() < 5.0);
    // MCP cancellation intentionally suppresses the cancelled response.
    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(!responses.iter().any(|v| v["id"] == 2));
    let lease = std::fs::File::options()
        .read(true)
        .write(true)
        .open(root.join("mock-lease"))
        .unwrap();
    assert!(lease.try_lock().is_ok());
    println!("selective MCP: active cancellation aborts and cleans up subprocess");
}
