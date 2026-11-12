use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};
const BIN: &str = env!("CARGO_BIN_EXE_code-rcl");

fn exchange(project: &std::path::Path, requests: Vec<Value>) -> Vec<Value> {
    let mut child = Command::new(BIN)
        .args(["mcp", "--project"])
        .arg(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ].into_iter().chain(requests) { writeln!(input, "{request}").unwrap(); }
    drop(input);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let value: Value =
                serde_json::from_str(line).expect("stdout must contain only JSON-RPC messages");
            assert_eq!(value["jsonrpc"], "2.0");
            value
        })
        .collect()
}
fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}
fn response(responses: &[Value], id: u64) -> &Value {
    responses.iter().find(|v| v["id"] == id).unwrap()
}

#[test]
fn c_and_cpp_filters_and_new_symbol_kinds_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plain.c"), "int c_value(void){return 1;}").unwrap();
    std::fs::write(dir.path().join("modern.cpp"),"template<class T> concept Number = requires(T v){v+v;}; struct Box { ~Box(){} }; int cpp_value(){return 2;}").unwrap();
    let out = exchange(
        dir.path(),
        vec![
            call(2, "code_rcl_graph", json!({"kinds":["contains"]})),
            call(
                3,
                "code_rcl_search",
                json!({"query":"c_value","languages":["c"]}),
            ),
            call(
                4,
                "code_rcl_search",
                json!({"query":"cpp_value","languages":["c++"]}),
            ),
        ],
    );
    assert!(
        response(&out, 1)["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("clangd")
    );
    for id in [2, 3, 4] {
        assert_eq!(
            response(&out, id)["result"]["isError"],
            false,
            "{}",
            response(&out, id)
        );
    }
    let nodes = response(&out, 2)["result"]["structuredContent"]["data"]["nodes"]
        .as_array()
        .unwrap();
    for kind in ["concept", "destructor"] {
        assert!(nodes.iter().any(|n| n["kind"] == kind));
    }
}

#[test]
fn graph_semantic_contract_selects_structural_edges_and_filters_guesses() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("C.java"),
        "class Base { int run(int x){return x;} } class Child extends Base { int run(int x){return x;} } class C { int pick(A x){return 1;} int pick(B x){return 2;} int start(){return pick(external());} }").unwrap();
    let out = exchange(
        dir.path(),
        vec![
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            call(
                3,
                "code_rcl_graph",
                json!({"kinds":["inherits","overrides","contains"]}),
            ),
            call(
                4,
                "code_rcl_graph",
                json!({"kinds":["calls"],"min_confidence":0.0,"no_sync":true}),
            ),
            call(
                5,
                "code_rcl_graph",
                json!({"kinds":["calls"],"min_confidence":0.5,"no_sync":true}),
            ),
        ],
    );
    let tools = response(&out, 2)["result"]["tools"].as_array().unwrap();
    let graph = tools
        .iter()
        .find(|t| t["name"] == "code_rcl_graph")
        .unwrap();
    assert!(graph["inputSchema"]["properties"].get("kinds").is_some());
    assert_eq!(
        graph["outputSchema"]["properties"]["schema_version"]["const"],
        2
    );
    for id in [3, 4, 5] {
        let result = &response(&out, id)["result"];
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(result["structuredContent"]["schema_version"], 2);
        assert_eq!(result["structuredContent"]["data"]["version"], 3);
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"]);
    }
    let data = |id| response(&out, id)["result"]["structuredContent"]["data"].clone();
    let structural = data(3);
    for kind in ["inherits", "overrides"] {
        assert!(
            structural["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == kind),
            "{structural}"
        );
    }
    let covered = data(4);
    assert!(
        covered["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["resolution"]["ambiguous"] == true)
    );
    let strict = data(5);
    assert!(
        strict["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["confidence"].as_f64().unwrap() >= 0.5)
    );
    assert!(
        strict["resolution_summary"]["below_min_confidence"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn explain_small_budget_preserves_identity_and_original_relation_counts() {
    let d = tempfile::tempdir().unwrap();
    let mut source = String::from("pub fn budget_target() {}\n");
    for i in 0..80 {
        source.push_str(&format!("pub fn caller_{i}() {{ budget_target(); }}\n"));
    }
    std::fs::write(d.path().join("lib.rs"), source).unwrap();
    let responses = exchange(
        d.path(),
        vec![
            call(
                2,
                "code_rcl_explain",
                json!({"symbol":"budget_target","max_output_kb":256}),
            ),
            call(
                3,
                "code_rcl_explain",
                json!({"symbol":"budget_target","max_output_kb":4}),
            ),
        ],
    );
    let full = &response(&responses, 2)["result"]["structuredContent"];
    let small_result = &response(&responses, 3)["result"];
    assert_eq!(small_result["isError"], false, "{small_result}");
    assert!(serde_json::to_vec(small_result).unwrap().len() <= 4096);
    let small = &small_result["structuredContent"];
    assert_eq!(small["status"], "ok");
    assert_eq!(small["metadata"]["truncated"], true);
    assert_eq!(small["data"].as_array().unwrap().len(), 1);
    let item = &small["data"][0];
    assert_eq!(item["symbol"], "budget_target");
    assert_eq!(item["path"], "lib.rs");
    let count = full["data"][0]["relations"]["callers"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(count, 80);
    assert_eq!(item["relation_counts"]["callers"], count);
    assert!(item["relations"]["callers"].as_array().unwrap().len() < count);
    let text: Value =
        serde_json::from_str(small_result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&text, small);
}

#[test]
fn schemas_and_runtime_reject_invalid_parameters() {
    let temp = tempfile::tempdir().unwrap();
    let out = exchange(
        temp.path(),
        vec![
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            call(3, "code_rcl_search", json!({"query":"x","json":true})),
            call(4, "code_rcl_impact", json!({"symbol":"x","diff":true})),
            call(5, "code_rcl_graph", json!({"depth":4294967297u64})),
            call(6, "code_rcl_search", json!({"query":"x","limit":"25"})),
        ],
    );
    assert_eq!(
        response(&out, 2)["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    let tools = response(&out, 2)["result"]["tools"].as_array().unwrap();
    let schema =
        |name: &str| &tools.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"];
    assert_eq!(
        schema("code_rcl_search")["properties"]["limit"]["default"],
        25
    );
    assert_eq!(
        schema("code_rcl_search")["properties"]["limit"]["maximum"],
        100.0
    );
    assert_eq!(
        schema("code_rcl_graph")["properties"]["max_nodes"]["default"],
        2000
    );
    assert_eq!(
        schema("code_rcl_dump")["properties"]["max_output_kb"]["default"],
        256
    );
    assert_eq!(
        schema("code_rcl_path")["properties"]["max_depth"]["default"],
        8
    );
    assert_eq!(
        schema("code_rcl_impact")["properties"]["depth"]["default"],
        2
    );
    assert!(schema("code_rcl_impact")["oneOf"].is_array());
    for id in 3..=6 {
        assert_eq!(response(&out, id)["error"]["code"], -32602);
    }
}

#[test]
fn clean_diff_and_tool_errors_do_not_corrupt_stdout() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
    for args in [
        vec!["init"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            Command::new("git")
                .current_dir(temp.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let out = exchange(
        temp.path(),
        vec![
            call(2, "code_rcl_impact", json!({"diff":true})),
            call(3, "code_rcl_explain", json!({"symbol":"missing"})),
        ],
    );
    let result = &response(&out, 2)["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["data"], json!([]));
    assert_eq!(
        serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        result["structuredContent"]
    );
    assert_eq!(response(&out, 3)["result"]["isError"], true);
}

#[test]
fn response_budget_omits_whole_files_and_reports_truncation() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.rs"),
        format!("fn main() {{}}\n// {}", "x".repeat(8000)),
    )
    .unwrap();
    let out = exchange(
        temp.path(),
        vec![call(
            2,
            "code_rcl_dump",
            json!({"target":"main","max_output_kb":4}),
        )],
    );
    let result = &response(&out, 2)["result"];
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["metadata"]["truncated"], true);
    assert!(
        result["structuredContent"]["data"]["files"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(serde_json::to_vec(result).unwrap().len() <= 4096);
}

#[test]
fn cancelled_queued_request_is_not_executed_and_ping_stays_available() {
    let temp = tempfile::tempdir().unwrap();
    let out = exchange(
        temp.path(),
        vec![
            call(2, "code_rcl_sync", json!({})),
            call(3, "code_rcl_sync", json!({})),
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}),
            json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
        ],
    );
    assert!(response(&out, 4)["result"].is_object());
    assert!(!out.iter().any(|r| r["id"] == 3));
}

#[test]
fn malformed_input_lifecycle_and_missing_project_are_reported_without_exiting() {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "{{broken-json").unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        call(4, "code_rcl_search", json!({"query":"x"})),
        json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert_eq!(responses[0]["id"], Value::Null);
    assert_eq!(response(&responses, 2)["error"]["code"], -32600);
    assert_eq!(response(&responses, 4)["result"]["isError"], true);
    assert!(response(&responses, 5)["result"].is_object());
}
#[test]
fn embedding_schema_errors_and_missing_models_remain_jsonrpc() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(".code-rcl")).unwrap();
    std::fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[embeddings]\nmodel_dir='missing'\n",
    )
    .unwrap();
    std::fs::write(d.path().join("lib.rs"), "pub fn alpha() {}\n").unwrap();
    let out = exchange(
        d.path(),
        vec![
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            call(
                3,
                "code_rcl_search",
                json!({"query":"alpha","retrieval":"bogus"}),
            ),
            call(
                4,
                "code_rcl_search",
                json!({"query":"alpha","retrieval":"semantic","strings":true}),
            ),
            call(
                5,
                "code_rcl_sync",
                json!({"embeddings":true,"no_sync":true}),
            ),
            call(
                6,
                "code_rcl_search",
                json!({"query":"alpha","retrieval":"hybrid"}),
            ),
            call(7, "code_rcl_search", json!({"query":"alpha"})),
            call(8, "code_rcl_sync", json!({"embeddings":true})),
            call(10, "code_rcl_sync", json!({"embeddings":true,"full":true})),
            call(11, "code_rcl_sync", json!({"full":true})),
            call(
                12,
                "code_rcl_sync",
                json!({"embeddings":true,"full":true,"no_sync":true}),
            ),
            json!({"jsonrpc":"2.0","id":9,"method":"ping"}),
        ],
    );
    for id in [3, 4, 5, 11, 12] {
        assert_eq!(response(&out, id)["error"]["code"], -32602);
    }
    for id in [6, 8, 10] {
        let result = &response(&out, id)["result"];
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["status"], "error");
        let v: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v, result["structuredContent"]);
    }
    let rebuild_error = response(&out, 10)["result"]["structuredContent"].to_string();
    assert!(!rebuild_error.contains("full requires"), "{rebuild_error}");
    #[cfg(feature = "embeddings")]
    assert!(rebuild_error.contains("model"), "{rebuild_error}");
    assert_eq!(
        response(&out, 7)["result"]["structuredContent"]["data"]["exact_symbols"][0]["name"],
        "alpha"
    );
    assert!(response(&out, 9)["result"].is_object());
    let tools = response(&out, 2)["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 9);
    let search = tools
        .iter()
        .find(|t| t["name"] == "code_rcl_search")
        .unwrap();
    assert_eq!(
        search["inputSchema"]["properties"]["retrieval"]["default"],
        "lexical"
    );
}
#[cfg(feature = "embeddings")]
#[test]
#[ignore = "requires CODE_RCL_EMBEDDING_MODEL_DIR; real-model CI job"]
fn real_model_mcp_index_and_semantic_contract() {
    let model = std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR").unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("lib.rs"),
        "pub fn cancel_request() {}\npub fn sort_names() {}\n",
    )
    .unwrap();
    let setup = Command::new(BIN)
        .args(["embeddings", "setup", "--model-dir", &model, "--project"])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let config_path = d.path().join(".code-rcl/config.toml");
    let mut config: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["embeddings"]
        .as_table_mut()
        .unwrap()
        .insert("batching".into(), toml::Value::String("length".into()));
    std::fs::write(config_path, toml::to_string(&config).unwrap()).unwrap();
    let out = exchange(
        d.path(),
        vec![
            call(2, "code_rcl_sync", json!({"embeddings":true})),
            call(6, "code_rcl_sync", json!({"embeddings":true,"full":true})),
            call(
                3,
                "code_rcl_search",
                json!({"query":"cancel request","retrieval":"semantic","max_output_kb":4}),
            ),
            call(
                4,
                "code_rcl_search",
                json!({"query":"cancel request","retrieval":"hybrid"}),
            ),
            call(
                7,
                "code_rcl_search",
                json!({"query":"cancel request","retrieval":"semantic"}),
            ),
            call(
                8,
                "code_rcl_search",
                json!({"query":"cancel_request when cancellation is needed","retrieval":"hybrid"}),
            ),
            json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
        ],
    );
    for id in [2, 6, 3, 4, 7, 8] {
        let result = &response(&out, id)["result"];
        assert_eq!(result["isError"], false, "{result}");
        let data = &result["structuredContent"];
        assert_eq!(data["metadata"]["embeddings"]["pending_files"], 0);
        assert_eq!(data["status"], "ok");
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&text, data);
    }
    let initial_generation = response(&out, 2)["result"]["structuredContent"]["metadata"]["embeddings"]["index_generation"].as_u64().unwrap();
    let rebuilt_generation = response(&out, 6)["result"]["structuredContent"]["metadata"]["embeddings"]["index_generation"].as_u64().unwrap();
    assert!(rebuilt_generation > initial_generation);
    assert!(
        !response(&out, 6)["result"]["structuredContent"]["metadata"]["resolution_mode"]
            .as_str()
            .unwrap()
            .contains("precise")
    );
    assert!(
        !response(&out, 7)["result"]["structuredContent"]["data"]["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        serde_json::to_vec(&response(&out, 3)["result"])
            .unwrap()
            .len()
            <= 4096
    );
    assert!(response(&out, 5)["result"].is_object());
    let mixed = &response(&out, 8)["result"]["structuredContent"];
    assert_eq!(mixed["metadata"]["retrieval"]["lexical_candidates"], 1);
    assert_eq!(mixed["data"]["matches"][0]["name"], "cancel_request");
    assert_eq!(mixed["data"]["matches"][0]["retrieval"], "hybrid");
    let stats = &response(&out, 2)["result"]["structuredContent"]["data"]["embeddings"];
    assert_eq!(stats["batching"], "length");
    assert_eq!(stats["embedded_chunks"], 2);
    assert_eq!(stats["unique_inferences"], 2);
    assert!(stats["scheduler_peak_bytes"].as_u64().unwrap() <= 64 * 1024 * 1024);
    let cli = Command::new(BIN)
        .args([
            "search",
            "cancel request",
            "--semantic",
            "--no-sync",
            "--json",
            "--project",
        ])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let mcp = &response(&out, 7)["result"]["structuredContent"]["data"]["matches"];
    for (cli, mcp) in cli["data"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .zip(mcp.as_array().unwrap())
    {
        for key in ["path", "name", "start_byte", "end_byte", "snippet"] {
            assert_eq!(cli[key], mcp[key]);
        }
        assert!(
            (cli["cosine_score"].as_f64().unwrap() - mcp["cosine_score"].as_f64().unwrap()).abs()
                < 1e-6
        );
    }
}

#[test]
fn semantic_rejects_explicit_lexical_matching_options() {
    let d = tempfile::tempdir().unwrap();
    let out = exchange(
        d.path(),
        vec![call(
            2,
            "code_rcl_search",
            json!({"query":"alpha","retrieval":"semantic","match_mode":"exact"}),
        )],
    );
    assert_eq!(response(&out, 2)["error"]["code"], -32602);
}

#[test]
fn progress_notifications_require_a_token_and_remain_jsonrpc() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(".code-rcl")).unwrap();
    std::fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[embeddings]\nmodel_dir='missing-model'\n",
    )
    .unwrap();
    let mut request = call(2, "code_rcl_sync", json!({"embeddings":true}));
    request["params"]["_meta"] = json!({"progressToken":"index-job"});
    let out = exchange(d.path(), vec![request]);
    assert!(
        out.iter().any(|v| v["method"] == "notifications/progress"
            && v["params"]["progressToken"] == "index-job")
    );
    let out = exchange(
        d.path(),
        vec![call(2, "code_rcl_sync", json!({"embeddings":true}))],
    );
    assert!(!out.iter().any(|v| v["method"] == "notifications/progress"));
}

#[test]
fn search_and_report_share_explicit_language_path_and_category_filters() {
    let d = tempfile::tempdir().unwrap();
    for folder in ["src", "tests", ".code-rcl"] {
        std::fs::create_dir(d.path().join(folder)).unwrap();
    }
    std::fs::write(d.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(d.path().join("tests/test.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[scope.categories]\nproduction=['src/**']\ntests=['tests/**']\n",
    )
    .unwrap();
    let out = exchange(
        d.path(),
        vec![
            call(
                2,
                "code_rcl_search",
                json!({"query":"alpha","languages":["rust"],"categories":["production"],"paths":["src/**"],"limit":1}),
            ),
            call(3, "code_rcl_report", json!({"categories":["tests"]})),
            call(
                4,
                "code_rcl_search",
                json!({"query":"alpha","paths":["../bad"]}),
            ),
        ],
    );
    assert_eq!(
        response(&out, 2)["result"]["structuredContent"]["data"]["exact_symbols"][0]["path"],
        "src/lib.rs"
    );
    assert_eq!(response(&out, 3)["result"]["isError"], false);
    assert_eq!(response(&out, 4)["error"]["code"], -32602);
    let cli = Command::new(BIN)
        .args([
            "search",
            "alpha",
            "--language",
            "rust",
            "--category",
            "production",
            "--path",
            "src/**",
            "--limit",
            "1",
            "--json",
            "--project",
        ])
        .arg(d.path())
        .output()
        .unwrap();
    assert!(cli.status.success());
    let v: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(
        v["data"],
        response(&out, 2)["result"]["structuredContent"]["data"]
    );
}

#[test]
fn error_operation_metadata_also_respects_response_budget() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(".code-rcl")).unwrap();
    std::fs::write(
        d.path().join(".code-rcl/config.toml"),
        "[embeddings]\nmodel_dir='missing-model'\n",
    )
    .unwrap();
    let out = exchange(
        d.path(),
        vec![call(
            2,
            "code_rcl_sync",
            json!({"embeddings":true,"max_output_kb":1}),
        )],
    );
    let result = &response(&out, 2)["result"];
    assert_eq!(result["isError"], true);
    assert!(
        serde_json::to_vec(result).unwrap().len() <= 1024,
        "{result}"
    );
}

#[test]
fn selective_contract_config_precedence_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def strong():\n    pass\ndef caller():\n    strong()\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join(".code-rcl")).unwrap();
    std::fs::write(
        dir.path().join(".code-rcl/config.toml"),
        "[precise]\nselective=true\n",
    )
    .unwrap();
    let out = exchange(
        dir.path(),
        vec![
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            call(3, "code_rcl_sync", json!({"full":true})),
            call(4, "code_rcl_sync", json!({"precise":false})),
            call(
                5,
                "code_rcl_sync",
                json!({"precise":true,"precise_selective":true}),
            ),
        ],
    );
    let tools = response(&out, 2)["result"]["tools"].as_array().unwrap();
    let sync = tools.iter().find(|v| v["name"] == "code_rcl_sync").unwrap();
    assert!(sync["inputSchema"]["properties"]["precise_selective"].is_object());
    assert_eq!(
        response(&out, 3)["result"]["isError"],
        false,
        "{}",
        response(&out, 3)
    );
    assert_eq!(
        response(&out, 3)["result"]["structuredContent"]["metadata"]["precision"]["requested_mode"],
        "selective"
    );
    assert_eq!(
        response(&out, 3)["result"]["structuredContent"]["metadata"]["resolution_mode"],
        "heuristic"
    );
    assert_eq!(response(&out, 4)["result"]["isError"], false);
    assert_eq!(response(&out, 5)["error"]["code"], -32602);
}
