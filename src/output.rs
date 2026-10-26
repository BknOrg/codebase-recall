use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

pub fn envelope(project: &Path, no_sync: bool, data: impl Serialize) -> Result<Value> {
    let serialize_timer = crate::operation::time("data_serialization");
    let data = serde_json::to_value(data)?;
    drop(serialize_timer);
    if no_sync {
        crate::operation::reason("source_unverified", json!({"no_sync":true}));
    }
    let (warnings, partial, omitted) = crate::operation::metadata();
    let (generation, precise, embeddings) = if crate::cache::bkndb_path(project).exists() {
        let db = crate::cache::CacheDb::open(project)?;
        let generation = db
            .meta_get("generation")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        (
            generation,
            db.meta_get("precise_generation")?.as_deref() == Some(generation.to_string().as_str()),
            crate::embeddings::status(&db, project, !no_sync)?,
        )
    } else {
        (0, false, serde_json::Value::Null)
    };
    Ok(json!({
        "schema_version": 1,
        "status": if partial { "partial" } else { "ok" },
        "data": data,
        "metadata": {
            "project": project.canonicalize()?.display().to_string(),
            "cache_generation": generation,
            "embeddings": embeddings,
            "resolution_mode": if crate::operation::graph_partial() { "mixed" } else if precise { "precise_cached" } else { "heuristic" },
            "freshness": if no_sync { "unverified" } else { "verified" },
            "warnings": warnings,
            "reasons": crate::operation::reasons(),
            "retrieval": crate::operation::retrieval_metadata(),
            "timings_ms": crate::operation::timings(),
            "truncated": omitted > 0,
            "omitted_count": omitted,
        }
    }))
}

pub fn json(project: &Path, no_sync: bool, data: impl Serialize) -> Result<String> {
    Ok(serde_json::to_string_pretty(&envelope(
        project, no_sync, data,
    )?)?)
}
