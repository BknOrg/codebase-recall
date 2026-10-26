//! Trim details before identities. Budget includes BOTH MCP result representations.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
fn size(v: &Value) -> Result<usize> {
    Ok(serde_json::to_vec(&json!({"structuredContent":v,"content":[{"type":"text","text":v.to_string()}],"isError":false}))?.len())
}
pub fn error_bound(value: &mut Value, budget: usize) {
    if size(value).is_ok_and(|n| n <= budget) {
        return;
    }
    let warning = value["metadata"]["warnings"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("tool failed")
        .chars()
        .take(120)
        .collect::<String>();
    let state = value["metadata"]["operation"]["state"].clone();
    *value = json!({"schema_version":1,"status":"error","data":null,"metadata":{"operation_state":state,"warnings":[warning,"Details omitted; increase max_output_kb or inspect embeddings status"],"truncated":true,"reasons":[{"kind":"response_budget"}]}});
    if size(value).is_ok_and(|n| n > budget) {
        *value = json!({"schema_version":1,"status":"error","data":null,"metadata":{"truncated":true,"warnings":["Tool failed; error details exceed max_output_kb. Increase the budget."],"reasons":[{"kind":"response_budget"}]}});
    }
}
fn item_count(value: &Value) -> usize {
    match value {
        Value::Array(items) => {
            items.iter().filter(|v| v.is_object()).count()
                + items.iter().map(item_count).sum::<usize>()
        }
        Value::Object(items) => items.values().map(item_count).sum(),
        _ => 0,
    }
}
pub fn bound(value: &mut Value, budget: usize, tool: &str) -> Result<()> {
    let _timer = crate::operation::time("response_budget");
    if size(value)? <= budget {
        return Ok(());
    }
    value["metadata"]["truncated"] = json!(true);
    if !value["metadata"]["reasons"].is_array() {
        value["metadata"]["reasons"] = json!([]);
    }
    let reason = json!({"kind":"response_budget","details":{"max_bytes":budget,"recovery":"increase max_output_kb or narrow the query"}});
    if !value["metadata"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&reason)
    {
        value["metadata"]["reasons"]
            .as_array_mut()
            .unwrap()
            .push(reason);
    }
    if matches!(tool, "code_rcl_explain" | "code_rcl_impact")
        && let Some(items) = value["data"].as_array_mut()
    {
        for item in items {
            if item.get("relation_counts").is_some() {
                continue;
            }
            let relation = if tool == "code_rcl_explain" {
                &item["relations"]
            } else {
                &*item
            };
            item["relation_counts"] = json!({"callers":relation["callers"].as_array().map_or(0,Vec::len),"callees":relation["callees"].as_array().map_or(0,Vec::len)});
        }
    }
    while size(value)? > budget {
        if remove_detail(&mut value["metadata"], "timings_ms") {
            value["metadata"]["trimmed_details"] =
                json!(value["metadata"]["trimmed_details"].as_u64().unwrap_or(0) + 1);
            continue;
        }
        let before = item_count(&value["data"]);
        let data = &mut value["data"];
        let changed = match tool {
            "code_rcl_explain" | "code_rcl_impact" => {
                let items = data
                    .as_array_mut()
                    .ok_or_else(|| anyhow::anyhow!("expected report array"))?;
                let mut changed = false;
                for item in items.iter_mut().rev() {
                    let relation = if tool == "code_rcl_explain" {
                        &mut item["relations"]
                    } else {
                        &mut *item
                    };
                    if pop_largest(relation, &["callers", "callees"]) {
                        changed = true;
                        break;
                    }
                    if pop_largest(item, &["members"])
                        || remove_detail(item, "doc")
                        || shorten(item, "signature", 256)
                    {
                        changed = true;
                        break;
                    }
                }
                if !changed && items.len() > 1 {
                    items.pop();
                    changed = true;
                }
                changed
            }
            "code_rcl_search" => {
                let mut changed = false;
                for field in [
                    "matches",
                    "exact_symbols",
                    "fuzzy_suggestions",
                    "string_literals",
                    "grep_matches",
                ] {
                    if let Some(items) = data[field].as_array_mut() {
                        for item in items.iter_mut().rev() {
                            if shorten_source(item, 160) || shorten(item, "signature", 256) {
                                changed = true;
                                break;
                            }
                            if let Some(locations) = item["supporting_locations"].as_array_mut() {
                                for location in locations.iter_mut().rev() {
                                    if shorten_source(location, 160) {
                                        changed = true;
                                        break;
                                    }
                                }
                            }
                            if changed || pop_largest(item, &["supporting_locations"]) {
                                changed = true;
                                break;
                            }
                        }
                    }
                    if changed {
                        break;
                    }
                }
                if !changed {
                    let total: usize = [
                        "matches",
                        "exact_symbols",
                        "fuzzy_suggestions",
                        "string_literals",
                        "grep_matches",
                    ]
                    .iter()
                    .map(|k| data[k].as_array().map_or(0, Vec::len))
                    .sum();
                    if total > 1 {
                        changed = pop_largest(
                            data,
                            &[
                                "matches",
                                "exact_symbols",
                                "fuzzy_suggestions",
                                "string_literals",
                                "grep_matches",
                            ],
                        );
                    }
                }
                changed
            }
            "code_rcl_graph" => {
                if pop_largest(data, &["edges"]) {
                    true
                } else if let Some(node) = data["nodes"].as_array_mut().and_then(Vec::pop) {
                    if let Some(edges) = data["edges"].as_array_mut() {
                        edges.retain(|e| e["source"] != node["id"] && e["target"] != node["id"]);
                    }
                    true
                } else {
                    false
                }
            }
            "code_rcl_dump" => pop_largest(data, &["files"]),
            _ => pop_largest(data, &["hubs", "subsystems", "bridges", "files", "matches"]),
        };
        ensure!(
            changed,
            "minimum response summary exceeds max_output_kb; result exists but cannot fit; increase the budget"
        );
        let omitted = before.saturating_sub(item_count(&value["data"]));
        value["metadata"]["omitted_count"] =
            json!(value["metadata"]["omitted_count"].as_u64().unwrap_or(0) + omitted as u64);
        if omitted == 0 {
            value["metadata"]["trimmed_details"] =
                json!(value["metadata"]["trimmed_details"].as_u64().unwrap_or(0) + 1);
        }
    }
    Ok(())
}
fn pop_largest(value: &mut Value, keys: &[&str]) -> bool {
    let key = keys
        .iter()
        .filter_map(|k| {
            value[*k]
                .as_array()
                .filter(|a| !a.is_empty())
                .map(|a| (*k, a.len()))
        })
        .max_by_key(|(_, n)| *n)
        .map(|(k, _)| k);
    key.and_then(|k| value[k].as_array_mut().and_then(Vec::pop))
        .is_some()
}
fn remove_detail(value: &mut Value, key: &str) -> bool {
    value.as_object_mut().and_then(|o| o.remove(key)).is_some()
}
fn shorten(value: &mut Value, key: &str, length: usize) -> bool {
    if let Some(text) = value[key].as_str().filter(|s| s.chars().count() > length) {
        value[key] = json!(text.chars().take(length).collect::<String>());
        true
    } else {
        false
    }
}
fn shorten_source(value: &mut Value, length: usize) -> bool {
    if !shorten(value, "snippet", length) {
        return false;
    }
    let snippet = value["snippet"].as_str().unwrap();
    let bytes = snippet.len() as u64;
    let newlines = snippet.bytes().filter(|b| *b == b'\n').count() as u64;
    if let Some(start) = value["start_byte"].as_u64() {
        value["end_byte"] = json!(start + bytes);
    }
    if value["end_line"].is_number()
        && let Some(line) = value["line"].as_u64()
    {
        value["end_line"] = json!(line + newlines);
    }
    value["snippet_truncated"] = json!(true);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explain_keeps_symbol_identity_and_counts_under_budget() {
        let mut v = json!({"schema_version":1,"status":"ok","data":[{"symbol":"big","path":"lib.rs","signature":"fn big()","doc":"x".repeat(4000),"members":[],"relations":{"callers":[{"snippet":"x".repeat(10000)}],"callees":[]}}],"metadata":{"omitted_count":0}});
        bound(&mut v, 2048, "code_rcl_explain").unwrap();
        assert_eq!(v["data"][0]["symbol"], "big");
        assert_eq!(v["data"][0]["relation_counts"]["callers"], 1);
        assert!(size(&v).unwrap() <= 2048);
        assert_eq!(v["metadata"]["truncated"], true);
        v["data"][0]["relations"]["callers"] = json!([]);
        v["metadata"]["timings_ms"] = json!(
            (0..200)
                .map(|n| (format!("phase_{n}"), json!(1.0)))
                .collect::<serde_json::Map<_, _>>()
        );
        bound(&mut v, 2048, "code_rcl_explain").unwrap();
        assert_eq!(v["data"][0]["relation_counts"]["callers"], 1);
        v["data"][0]["path"] = json!("long".repeat(4000));
        assert!(
            bound(&mut v, 1024, "code_rcl_explain")
                .unwrap_err()
                .to_string()
                .contains("result exists")
        );
    }
    #[test]
    fn search_shortens_unicode_detail_before_removing_last_candidate() {
        let mut v = json!({"data":{"matches":[{"name":"alpha","path":"lib.rs","cosine_score":0.5,"snippet":"日本語🙂".repeat(2000)}]},"metadata":{}});
        bound(&mut v, 4096, "code_rcl_search").unwrap();
        assert_eq!(v["data"]["matches"][0]["name"], "alpha");
        assert!(size(&v).unwrap() <= 4096);
    }
    #[test]
    fn search_budget_drops_timings_before_identity_and_preserves_freshness() {
        let diagnostics = (0..200)
            .map(|n| (format!("phase_{n}"), json!(1.0)))
            .collect::<serde_json::Map<_, _>>();
        let mut v = json!({"schema_version":1,"status":"ok","data":{"matches":[{"name":"alpha","path":"lib.rs","snippet":"fn alpha() {}"}]},"metadata":{"freshness":"verified","embeddings":{"pending_files":0},"timings_ms":diagnostics}});
        bound(&mut v, 4096, "code_rcl_search").unwrap();
        assert_eq!(v["data"]["matches"][0]["name"], "alpha");
        assert_eq!(v["metadata"]["freshness"], "verified");
        assert_eq!(v["metadata"]["embeddings"]["pending_files"], 0);
        assert!(v["metadata"].get("timings_ms").is_none());
        assert!(size(&v).unwrap() <= 4096);
    }
    #[test]
    fn clipped_primary_and_support_snippets_keep_exact_unicode_byte_spans() {
        let snippet = "日本語🙂\n".repeat(200);
        let source_item = json!({"name":"alpha","path":"lib.rs","snippet":snippet,"start_byte":20,"end_byte":20+snippet.len(),"line":4,"end_line":204});
        let mut primary = source_item.clone();
        primary["supporting_locations"] = json!([source_item]);
        let mut v = json!({"data":{"matches":[primary]},"metadata":{}});
        bound(&mut v, 4096, "code_rcl_search").unwrap();
        let primary = &v["data"]["matches"][0];
        for item in [primary, &primary["supporting_locations"][0]] {
            let clipped = item["snippet"].as_str().unwrap();
            assert!(snippet.starts_with(clipped));
            assert_eq!(item["end_byte"], 20 + clipped.len());
            assert_eq!(
                item["end_line"],
                4 + clipped.bytes().filter(|b| *b == b'\n').count()
            );
        }
        assert!(size(&v).unwrap() <= 4096);
    }
}

#[cfg(test)]
mod error_regressions {
    #[test]
    fn unicode_and_control_character_errors_fit_the_minimum_budget() {
        let mut v = serde_json::json!({"schema_version":1,"status":"error","data":null,"metadata":{"operation":{"state":"failed"},"warnings":["🙂\n\u{0000}".repeat(4000)]}});
        super::error_bound(&mut v, 1024);
        assert!(super::size(&v).unwrap() <= 1024);
        assert_eq!(v["status"], "error");
        assert_eq!(v["metadata"]["truncated"], true);
    }
}
