//! The two steps nearly every read-only command starts with — bring the cache
//! up to date, then turn it into a graph — kept in one place so `commands/*`
//! do not each rebuild the same sync arguments and graph query by hand.

use std::path::Path;

use anyhow::Result;

use crate::cache::CacheDb;
use crate::cli::{GraphQuery, PreciseArgs, SyncArgs};
use crate::commands::sync::{self, SyncStats};

/// Incrementally sync `project` with the defaults an implicit sync uses:
/// heuristic edges only, no report refresh.
pub fn auto_sync(db: &mut CacheDb, project: &Path) -> Result<SyncStats> {
    let args = SyncArgs {
        project: project.to_path_buf(),
        max_file_kb: 512,
        language: Vec::new(),
        no_report: true,
        precise: PreciseArgs::default(),
    };
    sync::sync_cache(db, &args)
}

/// A whole-project, both-scopes graph query over `kinds` — what impact, path,
/// digest hubs and the report all ask for, differing only in these inputs.
pub fn analysis_query(
    project: &Path,
    kinds: Vec<String>,
    no_sync: bool,
    precise: PreciseArgs,
) -> GraphQuery {
    GraphQuery {
        project: project.to_path_buf(),
        scope: "both".to_string(),
        kinds,
        path: None,
        focus: None,
        depth: 2,
        min_confidence: 0.0,
        include_external: false,
        max_nodes: 0,
        no_sync,
        precise,
    }
}

/// The `calls` + `imports` edge kinds used for architecture-level analysis.
pub fn call_and_import_kinds() -> Vec<String> {
    vec!["calls".to_string(), "imports".to_string()]
}
