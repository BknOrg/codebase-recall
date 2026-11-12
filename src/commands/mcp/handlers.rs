use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use crate::cache::CacheDb;
use crate::cli::{DigestArgs, ExplainArgs, GraphQuery, ImpactArgs, PathArgs, PreciseArgs};
use crate::commands::{digest, dump, explain, graph, impact, path, report, sync};

fn dispatch_tool(default_project: &Path, name: &str, args: &Value) -> Value {
    let result = match name {
        "code_rcl_digest" => execute_digest(default_project, args),
        "code_rcl_impact" => execute_impact(default_project, args),
        "code_rcl_path" => execute_path(default_project, args),
        "code_rcl_explain" => execute_explain(default_project, args),
        "code_rcl_report" => execute_report(default_project, args),
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

pub fn resolve_project(default_project: &Path, args: &Value) -> PathBuf {
    if let Some(p) = args.get("project").and_then(|v| v.as_str())
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    default_project.to_path_buf()
}

pub fn execute_digest(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let sub_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let doc_lines = args.get("doc_lines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let with_docs = args
        .get("with_docs")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let digest_args = DigestArgs {
        path: sub_path,
        project: Some(project),
        output: None,
        all,
        doc_lines,
        with_docs,
        json: as_json,
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };

    let report = digest::generate_digest(&digest_args)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&report)?)
    } else {
        Ok(digest::format_markdown_digest(&report))
    }
}

pub fn execute_impact(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let diff = args.get("diff").and_then(|v| v.as_bool()).unwrap_or(false);
    let symbol = args
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 8))
        .unwrap_or(2);

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .to_string();

    let kinds: Vec<String> = args
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "calls".to_string(),
                "imports".to_string(),
                "references".to_string(),
            ]
        });

    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);
    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let impact_args = ImpactArgs {
        symbol,
        diff,
        project,
        depth,
        direction,
        kinds,
        json: as_json,
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        precise: PreciseArgs {
            precise_selective: args["precise_selective"] == true,
            allow_heuristic_fallback: args
                .get("allow_heuristic_fallback")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            precise,
            precise_full: false,
            precise_timeout: None,
        },
    };

    let mut reports = impact::generate_reports(&impact_args)?;
    let trim = if diff {
        impact::trim_diff_reports(&mut reports, impact::DIFF_TARGET_LIMIT)
    } else {
        impact::DiffTrim::default()
    };
    if as_json {
        // Truncation metadata is recorded outside the data payload.
        crate::operation::omitted(trim.isolated + trim.over_limit);
        impact::render_json(&reports)
    } else {
        let mut out = impact::render_ascii(&reports);
        if let Some(note) = trim.note() {
            out.push_str(&format!(
                "
{note}
"
            ));
        }
        Ok(out)
    }
}

pub fn execute_path(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(from) = args.get("from").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'from'");
    };
    let Some(to) = args.get("to").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'to'");
    };

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("forward")
        .to_string();
    let max_depth = args
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 20))
        .unwrap_or(8);
    let kinds: Vec<String> = args
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "calls".to_string(),
                "imports".to_string(),
                "references".to_string(),
            ]
        });
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let path_args = PathArgs {
        from: from.to_string(),
        to: to.to_string(),
        project,
        kinds,
        direction,
        max_depth,
        json: as_json,
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        precise: precise_args(args),
    };

    let reports = path::generate_reports(&path_args)?;
    if as_json {
        path::render_json(&reports)
    } else {
        Ok(path::render_ascii(&reports))
    }
}

pub fn execute_explain(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'symbol'");
    };
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let explain_args = ExplainArgs {
        symbol: symbol.to_string(),
        project,
        json: as_json,
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        precise: precise_args(args),
    };

    let reports = explain::generate_reports(&explain_args)?;
    if as_json {
        explain::render_json(&reports)
    } else {
        Ok(explain::render_ascii(&reports))
    }
}

pub fn execute_report(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let data = report::generate_filtered(
        &project,
        args["no_sync"] == true,
        &crate::selection::Filters::from_mcp(args),
    )?;
    if as_json {
        Ok(serde_json::to_string_pretty(&data)?)
    } else {
        Ok(report::render_markdown(&data))
    }
}

pub fn execute_dump(default_project: &Path, args: &Value) -> Result<String> {
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

    let (_, files) = dump::relation_entries(
        &project,
        target,
        depth,
        max_size_kb,
        args["no_sync"] == true,
    )?;
    Ok(json!({"total_files": files.len(), "files": files}).to_string())
}

pub fn execute_search(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'query'");
    };
    let kind = args.get("kind").and_then(|v| v.as_str()).map(String::from);
    let exported = args
        .get("exported_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let strings = args
        .get("strings")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(25)
        .clamp(1, 100) as usize;
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let search_args = crate::cli::SearchArgs {
        filters: crate::selection::Filters::from_mcp(args),
        semantic: args["retrieval"] == "semantic",
        hybrid: args["retrieval"] == "hybrid",
        lexical: args["retrieval"] == "lexical",
        automatic: args["retrieval"] == "auto",
        match_mode: args
            .get("match_mode")
            .and_then(Value::as_str)
            .map(String::from),
        query: query.to_string(),
        project,
        kind,
        exported,
        strings,
        limit,
        json: as_json,
        no_fuzzy: false,
        no_grep: false,
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };

    let result = crate::commands::search::execute_search(&search_args)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&result)?)
    } else {
        Ok(crate::commands::search::render_search_result(&result))
    }
}

pub fn execute_sync(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let mut db = CacheDb::open(&project)?;
    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
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
        json: false,
        project: project.clone(),
        max_file_kb: None,
        language: languages,
        no_report: false,
        precise: PreciseArgs {
            precise_selective: args["precise_selective"] == true,
            allow_heuristic_fallback: args
                .get("allow_heuristic_fallback")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            precise,
            precise_full: (precise || args["precise_selective"] == true) && full,
            precise_timeout: None,
        },
    };

    crate::progress::phase("source_sync");
    let stats = sync::sync_cache(&mut db, &sync_args)?;
    let mut embedding_stats = Value::Null;
    crate::progress::index(crate::embeddings::status(&db, &project, true)?);
    if args["embeddings"] == true {
        embedding_stats = crate::embeddings::index_prepared(&mut db, &project, full)?;
    }
    let (total_symbols, total_imports) = db.totals()?;

    if sync_args.precise.requested() {
        crate::progress::phase("precise_resolution");
        let precise_stats = sync::run_precise(&mut db, &sync_args)?;
        sync::report_precise(&precise_stats);
        if !precise_stats.errors.is_empty() {
            if !sync_args.precise.allow_heuristic_fallback && !sync_args.precise.precise_selective {
                anyhow::bail!("precise failed: {}", precise_stats.errors.join("; "));
            }
            crate::operation::partial();
        }
    }

    // The report reopens the cache to build its graph, and bkndb allows one
    // open handle per file, so release ours first.
    drop(db);
    if let Err(e) = report::refresh_after_sync(&project, &stats, sync_args.precise.requested()) {
        eprintln!("warning: could not refresh the code report: {e:#}");
    }

    Ok(
        json!({"stats": stats, "total_symbols": total_symbols, "total_imports": total_imports,"embeddings":embedding_stats})
            .to_string(),
    )
}

pub fn execute_graph(default_project: &Path, args: &Value) -> Result<String> {
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

    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let query = GraphQuery {
        project,
        scope,
        kinds: args["kinds"]
            .as_array()
            .map(|kinds| {
                kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec!["imports".into(), "calls".into(), "contains".into()]),
        path: None,
        focus,
        depth: Some(depth),
        min_confidence: Some(args["min_confidence"].as_f64().unwrap_or(0.0) as f32),
        include_external: Some(false),
        max_nodes: Some(
            args.get("max_nodes")
                .and_then(Value::as_u64)
                .unwrap_or(2000) as usize,
        ),
        no_sync: args
            .get("no_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        precise: PreciseArgs {
            precise_selective: args["precise_selective"] == true,
            allow_heuristic_fallback: args
                .get("allow_heuristic_fallback")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            precise,
            precise_full: false,
            precise_timeout: None,
        },
    };

    let code_graph = graph::build_graph(&query)?;
    Ok(serde_json::to_string_pretty(&code_graph)?)
}

fn precise_args(args: &Value) -> PreciseArgs {
    PreciseArgs {
        precise_selective: args["precise_selective"] == true,
        precise: args["precise"] == true,
        allow_heuristic_fallback: args["allow_heuristic_fallback"] == true,
        ..Default::default()
    }
}

pub fn configured_args(default_project: &Path, name: &str, args: &Value) -> Result<Value> {
    anyhow::ensure!(args.is_object(), "tool arguments must be an object");
    let mut configured = args.clone();
    let project = resolve_project(default_project, args);
    if matches!(name, "code_rcl_sync" | "code_rcl_graph")
        && args["no_sync"] != true
        && args.get("precise").is_none()
        && args.get("precise_selective").is_none()
        && project.is_dir()
    {
        let cfg = crate::config::ProjectConfig::load(&project)?;
        configured["precise"] = json!(cfg.precise.enabled);
        configured["precise_selective"] = json!(cfg.precise.selective);
    }
    Ok(configured)
}

pub fn call_tool(default_project: &Path, name: &str, args: &Value) -> Value {
    let mut operation_snapshot = Value::Null;
    let result = (|| -> Result<Value> {
        let configured = configured_args(default_project, name, args)?;
        let mut args = super::inputs::validate(name, &configured)?;
        anyhow::ensure!(
            args["project"].is_string() || !default_project.as_os_str().is_empty(),
            "project is required when server has no --project"
        );
        let project = resolve_project(default_project, &args);
        anyhow::ensure!(project.is_dir(), "project must be an existing directory");
        let budget = args["max_output_kb"].as_u64().unwrap_or(256) as usize * 1024;
        args["json"] = Value::Bool(true);
        let mut observer = if name == "code_rcl_sync" && args["embeddings"] == true {
            Some(crate::progress::Guard::start(&project, None)?)
        } else {
            None
        };
        let response = dispatch_tool(&project, name, &args);
        if response["isError"] == true {
            crate::progress::update(|v| v["error"] = response["content"][0]["text"].clone());
        }
        let operation = observer
            .as_mut()
            .map(|guard| {
                guard.finish(if response["isError"] == true {
                    if crate::operation::cancelled() {
                        "cancelled"
                    } else {
                        "failed"
                    }
                } else {
                    "completed"
                })
            })
            .transpose()?;
        operation_snapshot = operation.clone().unwrap_or(Value::Null);
        anyhow::ensure!(
            response["isError"] != true,
            "{}",
            response["content"][0]["text"]
                .as_str()
                .unwrap_or("tool failed")
        );
        let text = response["content"][0]["text"].as_str().unwrap_or("");
        let data = serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({"content": text}));
        let mut envelope = crate::output::envelope(&project, args["no_sync"] == true, data)?;
        if let Some(operation) = operation {
            envelope["metadata"]["operation"] = operation;
        }
        if args["diff"] == true {
            envelope["metadata"]["untracked_included"] = json!(false);
        }
        super::budget::bound(&mut envelope, budget, name)?;
        let returned = envelope["data"].as_array().map(Vec::len).or_else(|| {
            envelope["data"].as_object().map(|o| {
                o.values()
                    .filter_map(Value::as_array)
                    .map(Vec::len)
                    .sum::<usize>()
            })
        });
        envelope["metadata"]["returned_count"] = json!(returned);
        if envelope["metadata"].get("timings_ms").is_some() {
            envelope["metadata"]["timings_ms"] = crate::operation::timings();
        }
        super::budget::bound(&mut envelope, budget, name)?;
        Ok(envelope)
    })();
    let (mut envelope, error) = match result {
        Ok(value) => (value, false),
        Err(error) => (
            json!({"schema_version":2,"status":"error","data":null,"metadata":{"operation":operation_snapshot,"warnings":[format!("{error:#}")]}}),
            true,
        ),
    };
    if error {
        super::budget::error_bound(
            &mut envelope,
            args["max_output_kb"].as_u64().unwrap_or(256).clamp(1, 2048) as usize * 1024,
        );
    }
    json!({"content":[{"type":"text","text":envelope.to_string()}],"structuredContent":envelope,"isError":error})
}
