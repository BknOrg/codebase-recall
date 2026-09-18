use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub target_symbol: String,
    pub target_id: String,
    pub target_kind: String,
    pub target_path: Option<String>,
    pub direction: String,
    pub direct_callers_count: usize,
    pub total_callers_count: usize,
    pub callers: Vec<ImpactItem>,
    pub direct_callees_count: usize,
    pub total_callees_count: usize,
    pub callees: Vec<ImpactItem>,
    pub total_affected_count: usize,
    pub total_affected_files: usize,
    pub risky_affected_count: usize,
    /// Subsystem (community) the target belongs to, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_community: Option<String>,
    pub max_depth_reached: u32,
}

#[derive(Debug, Serialize)]
pub struct ImpactItem {
    pub id: String,
    pub label: String,
    pub kind: String,
    /// File that defines this item.
    pub path: Option<String>,
    /// Line of the relation, in `site_path` (not necessarily in `path`).
    pub line: Option<i64>,
    /// File the relation's `line` lives in: the item's own file for a caller,
    /// but the *target's* file for a callee, whose definition is elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_path: Option<String>,
    pub edge_kind: String,
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub depth: u32,
    pub is_exported: bool,
    pub low_confidence: bool,
    /// True when this item sits in a different subsystem (community) than the target;
    /// falls back to "different top-level directory" when either side has no community.
    pub crosses_module: bool,
    /// Subsystem this item belongs to, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    pub callers: Vec<ImpactItem>,
}
